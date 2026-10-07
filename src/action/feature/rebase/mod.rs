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

use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report};
use crate::git;

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

mod repo;
mod types;

use repo::rebase_one_repo;
pub use types::{BaseSource, RebaseInput, RebaseOutcome, RebaseStatus, RepoRebase};

/// Rebase the selected promoted repos of `input.name` onto their target base.
///
/// Blocked when the feature does not exist. Per-repo problems are warnings on
/// a clean report — skipped (dirty, or no worktree) and conflicted repos
/// continue the batch, exactly like `deliver`'s best-effort pushes.
pub fn rebase(ctx: &Ctx, input: RebaseInput) -> Outcome<RebaseOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

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
        let (result, repo_warnings, did_collapse) = rebase_one_repo(
            &git,
            &layout,
            &feature,
            repo_name,
            promotion,
            &manifest,
            onto.as_ref(),
            input.offline,
        )?;
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
