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
//! # Per-repo, never a batch abort
//!
//! A dirty worktree is skipped with a warning — rebasing over uncommitted work
//! is how it gets lost. A rebase that stops (a conflict, or any other git
//! refusal) is aborted with `git rebase --abort` and reported as conflicted,
//! and the next repo is tried. The report carries one status per repo:
//! `rebased`, `skipped`, or `conflicted`.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git};
use crate::infra::fs;
use crate::store::layout::Layout;

use super::super::{discover_hall, read_manifest};
use super::base;
use crate::action::Ctx;

/// What `ivar feature rebase` needs.
#[derive(Debug, Clone)]
pub struct RebaseInput {
    /// The feature's name.
    pub name: String,
    /// Collapse the base: rebase every selected repo onto this branch,
    /// unvalidated, and record it as the declared base for each repo that
    /// actually lands there.
    pub onto: Option<String>,
    /// Promoted repos to rebase; empty means every promoted repo.
    pub repos: Vec<String>,
    /// Rebase onto the local base ref and make no network call.
    pub offline: bool,
}

/// What happened to one promoted repo's worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RebaseStatus {
    /// The rebase completed — the worktree's branch now sits on its target base's tip.
    Rebased,
    /// The repo was not rebased (dirty worktree, or no worktree to rebase).
    Skipped,
    /// The rebase stopped and was aborted; the worktree is untouched.
    Conflicted,
}

/// Where the tip a repo was rebased onto came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseSource {
    /// The remote's tip, fetched into the worktree's `FETCH_HEAD`.
    Remote,
    /// The hall's local base ref.
    Local,
}

/// One promoted repo's rebase result.
#[derive(Debug, Clone, Serialize)]
pub struct RepoRebase {
    pub repo: RepoName,
    pub status: RebaseStatus,
    /// The base this repo was rebased (or tried to rebase) onto; `None` only
    /// when the repo is not in `ivar.json`.
    pub onto: Option<BranchName>,
    /// Where `onto`'s tip came from; `None` when the repo was skipped
    /// before any rebase ran.
    pub base_source: Option<BaseSource>,
}

/// What `ivar feature rebase` did.
#[derive(Debug, Clone, Serialize)]
pub struct RebaseOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature whose repos were rebased.
    pub feature: FeatureName,
    /// The feature branch every promoted worktree is on.
    pub branch: String,
    /// One entry per selected promoted repo, in name order.
    pub repos: Vec<RepoRebase>,
}

impl WriteHuman for RebaseOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Rebased feature `{}` (branch: {}) in {}:",
            self.feature, self.branch, self.root
        )?;
        if self.repos.is_empty() {
            writeln!(w, "  no repos promoted")?;
        }
        for repo in &self.repos {
            let status = match repo.status {
                RebaseStatus::Rebased => "rebased",
                RebaseStatus::Skipped => "skipped",
                RebaseStatus::Conflicted => "conflicted",
            };
            match (&repo.onto, repo.base_source) {
                (Some(onto), Some(source)) => writeln!(
                    w,
                    "  {}  {status}  onto {onto} ({})",
                    repo.repo,
                    match source {
                        BaseSource::Remote => "remote",
                        BaseSource::Local => "local",
                    }
                )?,
                _ => writeln!(w, "  {}  {status}", repo.repo)?,
            }
        }
        Ok(())
    }
}

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

/// A repo left untouched before any rebase ran.
fn skipped(
    repo_name: &RepoName,
    onto: Option<BranchName>,
    code: &'static str,
    what: &str,
) -> (RepoRebase, Vec<Warning>, bool) {
    (
        RepoRebase {
            repo: repo_name.clone(),
            status: RebaseStatus::Skipped,
            onto,
            base_source: None,
        },
        vec![Warning::new(code, repo_name.as_str(), what)],
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn rebase_one_repo(
    git: &impl Git,
    layout: &Layout,
    feature: &Feature,
    repo_name: &RepoName,
    promotion: &crate::domain::feature::Promotion,
    manifest: &crate::store::manifest::Manifest,
    onto: Option<&BranchName>,
    offline: bool,
) -> Result<(RepoRebase, Vec<Warning>, bool), Failure> {
    let worktree = layout.repo_worktree(repo_name, &feature.branch);
    let Some(manifest_repo) = manifest
        .repos()
        .iter()
        .find(|repo| repo.name() == repo_name)
    else {
        return Ok(skipped(
            repo_name,
            None,
            "rebase.repo_not_in_manifest",
            "not in ivar.json; nothing to rebase onto",
        ));
    };
    let target = onto
        .cloned()
        .unwrap_or_else(|| base::resolve(feature, promotion, manifest_repo.default_branch()));
    if !fs::is_dir(&worktree)? {
        return Ok(skipped(
            repo_name,
            Some(target),
            "rebase.no_worktree",
            "no worktree materialised for this repo",
        ));
    }
    // Rebase over uncommitted work is how it gets lost — a dirty worktree is
    // skipped, never rebased around, and never costs a network call.
    if git.worktree_dirty(&worktree)? {
        return Ok(skipped(
            repo_name,
            Some(target),
            "rebase.dirty",
            "worktree has uncommitted changes; commit or stash them first",
        ));
    }
    let (from, source, fallback) = rebase_from(
        git,
        layout,
        repo_name,
        manifest_repo.url(),
        &worktree,
        &target,
        offline,
    );
    let mut warnings: Vec<Warning> = fallback.into_iter().collect();
    match git.rebase_branch(&worktree, &from) {
        Ok(()) => Ok((
            RepoRebase {
                repo: repo_name.clone(),
                status: RebaseStatus::Rebased,
                onto: Some(target),
                base_source: Some(source),
            },
            warnings,
            onto.is_some(),
        )),
        Err(git::Error::Refused { .. }) => {
            warnings.push(if let Err(abort) = git.abort_rebase(&worktree) {
                Warning::new(
                    "rebase.abort_failed",
                    repo_name.as_str(),
                    format!("could not abort the stopped rebase: {abort}"),
                )
            } else {
                Warning::new(
                    "rebase.conflicted",
                    repo_name.as_str(),
                    "rebase stopped (likely a conflict) and was aborted",
                )
            });
            Ok((
                RepoRebase {
                    repo: repo_name.clone(),
                    status: RebaseStatus::Conflicted,
                    onto: Some(target),
                    base_source: Some(source),
                },
                warnings,
                false,
            ))
        }
        Err(other) => Err(other.into()),
    }
}

/// The revision to rebase onto, where it came from, and the warning a
/// fallback carries. Runs only for a clean worktree.
fn rebase_from(
    git: &impl Git,
    layout: &Layout,
    repo_name: &RepoName,
    url: &str,
    worktree: &Utf8Path,
    target: &BranchName,
    offline: bool,
) -> (String, BaseSource, Option<Warning>) {
    if offline {
        return (target.to_string(), BaseSource::Local, None);
    }
    let unreachable = |detail: String| {
        Warning::new(
            "rebase.remote_unreachable",
            repo_name.as_str(),
            format!(
                "could not read `{target}` from the remote ({detail}); rebased onto the local `{target}`, which may be stale. Re-run when the remote is reachable"
            ),
        )
    };
    match git.remote_branch_tip(&layout.repo_bare(repo_name), url, target.as_str()) {
        Ok(Some(_)) => match git.fetch_branch(worktree, target.as_str()) {
            Ok(()) => ("FETCH_HEAD".to_owned(), BaseSource::Remote, None),
            Err(error) => (
                target.to_string(),
                BaseSource::Local,
                Some(unreachable(error.to_string())),
            ),
        },
        Ok(None) => (target.to_string(), BaseSource::Local, None),
        Err(error) => (
            target.to_string(),
            BaseSource::Local,
            Some(unreachable(error.to_string())),
        ),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/rebase.rs"]
mod tests;
