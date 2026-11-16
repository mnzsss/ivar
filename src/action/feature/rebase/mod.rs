//! `ivar feature rebase <name>` — rebase every promoted repo's feature-branch
//! worktree onto its base.
//!
//! The point of a rebase here is to bring a feature's work up to date with the
//! work that landed on its base since the feature branched. Each promoted
//! repo's worktree (on the feature branch) is replayed on top of that repo's
//! effective base — [`crate::action::feature::base::resolve`]: the base
//! `promote` recorded for that repo, or, for a promotion recorded before that
//! field existed, the feature's declared base against `default_branch` from
//! `ivar.json`.
//!
//! # Which tip: the remote's, unless `--offline`
//!
//! `deliver` refuses a branch that is not built on the base's *remote* tip
//! (`check_pr_base` asks `ls-remote`). So, by default, the base tip comes
//! from the remote too: `remote_branch_tip` against the manifest URL, then
//! `fetch_branch` into the worktree's own `FETCH_HEAD`, and the rebase
//! replays onto that. No local branch moves except the feature branch
//! (the fetch also refreshes the remote-tracking `refs/remotes/origin/<base>`,
//! which is not a branch); the local default branch, checked out in the base
//! worktree, is left alone.
//!
//! A base the remote does not carry (an unpublished parent feature) is
//! rebased onto locally, silently. A remote that does not answer also
//! falls back to the local ref, but with a `rebase.remote_unreachable`
//! warning, because that ref may be stale. `--offline` skips the remote
//! entirely.
//!
//! # `--repo`: a subset
//!
//! `--repo` names the promoted repos to rebase. An unknown name is refused
//! before anything moves. The mutability preflight and `--onto`'s base
//! collapse cover only the selection.
//!
//! # `--onto`: collapsing the base
//!
//! `--onto <branch>` is the verb to use once a feature's own base (typically
//! another feature, now delivered) has landed. Every selected repo rebases
//! onto `<branch>` instead of its own resolved base. `Promotion::base` is
//! rewritten to `<branch>` only for the repos that actually land there.
//!
//! # Per-repo, never a batch abort
//!
//! A dirty worktree is skipped with a warning — rebasing over uncommitted work
//! is how it gets lost. A rebase that stops (a conflict, or any other git
//! refusal) is aborted with `git rebase --abort` and reported as conflicted,
//! and the next repo is tried. The report carries one status per repo:
//! `rebased`, `skipped`, or `conflicted`.
//!
//! # `--keep-conflicts`, `--continue`, `--abort`
//!
//! Aborting is the default, so a batch never leaves repos half-rebased. With
//! `--keep-conflicts` a rebase that stops on a conflict is instead left in
//! progress: the repo is `conflicted` with `in_progress` set and the unmerged
//! paths listed, and the next repo is tried. Whoever resolves it does so in the
//! worktree, `git add`s the files, and runs `--continue`; `--abort` gives up
//! and restores the branch. Neither needs state beyond git's own: the rebase
//! in progress *is* the state. A refusal with no rebase in progress (an
//! unknown revision, say) is not a conflict and is never reported as one.
//! `--onto` cannot be combined with `--keep-conflicts`: the declared base is
//! only recorded for repos that finished, and a stopped rebase has not.
//!
//! A worktree left in progress is skipped by a plain `feature rebase`, with a
//! warning that names `--continue` and `--abort`.

use camino::Utf8Path;

use crate::domain::delivery_event::{DeliveryEvent, DeliveryScope};
use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report};
use crate::git::{self, Git};
use crate::infra::fs;

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;
use crate::action::memory::delivery::Emitter;

mod repo;
mod types;

use repo::{abort_one_repo, continue_one_repo, rebase_one_repo};
pub use types::{BaseSource, RebaseInput, RebaseMode, RebaseOutcome, RebaseStatus, RepoRebase};

/// Rebase the selected promoted repos of `input.name` onto their target base.
///
/// Blocked when the feature does not exist. Per-repo problems are warnings on
/// a clean report — skipped (dirty, or no worktree) and conflicted repos
/// continue the batch, exactly like `deliver`'s best-effort pushes.
pub fn rebase(ctx: &Ctx, input: RebaseInput) -> Outcome<RebaseOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;
    let emitter = ctx.emitter(&layout);

    let name = FeatureName::new(input.name)?;
    let feature = Feature::read(&layout, &name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
        .expected("an existing feature")
        .actual(format!("`{name}` has no feature.json"))
        .fix(FixAction::safe(
            "feature.create_first",
            format!("Create it first with `ivar feature create {name}`."),
        ))
    })?;

    let selected = select_repos(&feature, &input.repos)?;
    for repo in &selected {
        super::mutation::ensure_promotion_mutable(&layout, &feature, repo)?;
    }

    refuse_onto_with_resume(input.onto.as_deref(), input.mode)?;
    let onto = match input.onto {
        Some(raw) => Some(BranchName::new(raw)?),
        None => None,
    };

    let mut repos = Vec::new();
    let mut warnings = Vec::new();
    // Repos `--onto` actually rebased onto the new target — the only ones
    // whose declared base is safe to rewrite once the loop is done. A repo
    // that was skipped or conflicted keeps its old declared base: recording
    // a target its worktree was never moved onto would be a lie the next
    // rebase or delivery would believe.
    let mut collapsed: Vec<RepoName> = Vec::new();

    for repo_name in &selected {
        let Some(promotion) = feature.promotions.get(repo_name) else {
            continue;
        };
        let worktree = layout.repo_worktree(repo_name, &feature.branch);
        // Resuming, HEAD is already detached mid-rebase; the branch's tip
        // before the rebase began is what git recorded as `orig-head`.
        let sha_before = match input.mode {
            RebaseMode::Run { .. } => git.head_commit(&worktree).ok(),
            RebaseMode::Continue | RebaseMode::Abort => original_head(&git, &worktree),
        };
        let (result, repo_warnings, did_collapse) = match input.mode {
            RebaseMode::Run { keep_conflicts } => rebase_one_repo(
                &git,
                &layout,
                &feature,
                repo_name,
                promotion,
                &manifest,
                onto.as_ref(),
                input.offline,
                keep_conflicts,
            )?,
            RebaseMode::Continue => continue_one_repo(&git, &layout, &feature, repo_name)?,
            RebaseMode::Abort => abort_one_repo(&git, &layout, &feature, repo_name)?,
        };
        if matches!(
            result.status,
            RebaseStatus::Rebased | RebaseStatus::Conflicted
        ) {
            emit_rebased(
                &emitter, &git, &feature, repo_name, &worktree, &result, sha_before,
            );
        }
        repos.push(result);
        warnings.extend(repo_warnings);
        if did_collapse {
            collapsed.push(repo_name.clone());
        }
    }
    repos.sort_by(|a, b| a.repo.cmp(&b.repo));

    // Collapse the base only where the worktree actually landed on it — the
    // declaration and the worktree move together, or neither moves.
    if let Some(onto) = &onto
        && !collapsed.is_empty()
    {
        Feature::update(&layout, &name, |stored| {
            for repo_name in &collapsed {
                if let Some(promotion) = stored.promotions.get_mut(repo_name) {
                    promotion.base = Some(onto.clone());
                }
            }
            Ok(())
        })?;
    }

    Ok(Report::with_warnings(
        RebaseOutcome {
            root: layout.root().to_path_buf(),
            feature: name,
            branch: feature.branch.to_string(),
            repos,
        },
        warnings,
    ))
}

/// Record the `delivery.rebased` event for a repo that rebased or stopped.
fn emit_rebased(
    emitter: &Emitter<'_>,
    git: &impl Git,
    feature: &Feature,
    repo_name: &RepoName,
    worktree: &Utf8Path,
    result: &RepoRebase,
    sha_before: Option<String>,
) {
    // A rebase left in progress has only a detached HEAD midway through the
    // replay; recording it as the result would be false.
    let sha_after = if result.in_progress {
        None
    } else {
        git.head_commit(worktree).ok()
    };
    emitter.emit(
        DeliveryScope::for_repo(
            feature,
            repo_name.as_str(),
            sha_after
                .as_deref()
                .or(sha_before.as_deref())
                .unwrap_or_default(),
        ),
        DeliveryEvent::Rebased {
            conflicted: result.status == RebaseStatus::Conflicted,
            onto: result.onto.as_ref().map(ToString::to_string),
            base_source: result.base_source.map(|source| match source {
                BaseSource::Remote => "remote".to_owned(),
                BaseSource::Local => "local".to_owned(),
            }),
            sha_before,
            sha_after,
        },
    );
}

/// `--onto` only makes sense for a rebase that runs to completion or aborts:
/// the declared base is recorded for repos whose rebase finished, and a
/// stopped or resumed one has not.
fn refuse_onto_with_resume(onto: Option<&str>, mode: RebaseMode) -> Result<(), Failure> {
    if onto.is_none() || mode == RebaseMode::default() {
        return Ok(());
    }
    Err(Failure::blocked(
        "rebase.onto_unsupported",
        "`--onto` cannot be combined with `--keep-conflicts`, `--continue` or `--abort`",
    )
    .expected("`--onto` on a rebase that runs to completion or aborts")
    .actual("a mode that leaves or resumes a rebase in progress")
    .fix(FixAction::safe(
        "rebase.drop_onto",
        "Drop `--onto`: the declared base is only recorded for repos whose rebase finished.",
    )))
}

/// The commit the branch sat on before the rebase in progress in `worktree`
/// began, as git recorded it in `orig-head`; `None` when there is no such
/// rebase or git left no record.
fn original_head(git: &impl Git, worktree: &Utf8Path) -> Option<String> {
    let git_dir = git.worktree_git_dir(worktree).ok()?;
    ["rebase-merge", "rebase-apply"]
        .iter()
        .find_map(|dir| {
            fs::read_text(&git_dir.join(dir).join("orig-head"))
                .ok()
                .flatten()
        })
        .map(|sha| sha.trim().to_owned())
        .filter(|sha| !sha.is_empty())
}

/// The promoted repos `requested` names, deduplicated and in name order, or
/// every promoted repo when it is empty. An unknown name is refused before
/// anything moves.
fn select_repos(feature: &Feature, requested: &[String]) -> Result<Vec<RepoName>, Failure> {
    if requested.is_empty() {
        return Ok(feature.promotions.keys().cloned().collect());
    }
    let mut selected = std::collections::BTreeSet::new();
    for raw in requested {
        let repo = RepoName::new(raw.as_str())?;
        if !feature.promotions.contains_key(&repo) {
            let promoted = feature
                .promotions
                .keys()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(Failure::blocked(
                "feature.not_promoted",
                format!("`{repo}` is not promoted into `{}`", feature.name),
            )
            .expected("a repo currently promoted into this feature")
            .actual(if promoted.is_empty() {
                "no repo is promoted".to_owned()
            } else {
                format!("promoted: {promoted}")
            })
            .fix(FixAction::safe(
                "feature.promote_first",
                format!(
                    "Pass a promoted repo to `--repo`, or run `ivar feature promote {} {repo}` first.",
                    feature.name
                ),
            )));
        }
        selected.insert(repo);
    }
    Ok(selected.into_iter().collect())
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/feature/rebase.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../../tests/unit/action/feature/rebase_events.rs"]
mod events_tests;
