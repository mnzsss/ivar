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
//! # `--onto`: collapsing the base
//!
//! `--onto <branch>` is the verb for once a feature's own base — typically
//! another feature, now delivered — has landed. Every promoted repo rebases
//! onto `<branch>` instead of its own individually-resolved base, and
//! `Promotion::base` is rewritten to `<branch>` only for the repos that
//! actually land there: the declaration and the worktree move together, or
//! neither moves — a repo skipped (dirty, missing) or conflicted keeps its
//! old declared base, never a target its worktree was never rebased onto.
//!
//! No network call is introduced either way: `rebase` is the one destructive
//! verb that runs offline with confidence, and stays that way.
//!
//! # Per-repo, never a batch abort
//!
//! A dirty worktree is skipped with a warning — rebasing over uncommitted work
//! is how it gets lost. A rebase that stops (a conflict, or any other git
//! refusal) is aborted with `git rebase --abort` and reported as conflicted,
//! and the next repo is tried. The report carries one status per repo:
//! `rebased`, `skipped`, or `conflicted`.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git};
use crate::infra::fs;

use super::super::{discover_hall, read_manifest};
use super::base;
use crate::action::Ctx;

/// What `ivar feature rebase` needs.
#[derive(Debug, Clone)]
pub struct RebaseInput {
    /// The feature's name.
    pub name: String,
    /// Collapse the base: rebase every promoted repo onto this branch,
    /// unvalidated, and record it as the declared base for each repo that
    /// actually lands there.
    pub onto: Option<String>,
}

/// What happened to one promoted repo's worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RebaseStatus {
    /// The rebase completed — the worktree's branch now sits on the default
    /// branch's tip.
    Rebased,
    /// The repo was not rebased (dirty worktree, or no worktree to rebase).
    Skipped,
    /// The rebase stopped and was aborted; the worktree is untouched.
    Conflicted,
}

/// One promoted repo's rebase result.
#[derive(Debug, Clone, Serialize)]
pub struct RepoRebase {
    /// The repo.
    pub repo: RepoName,
    /// What happened to it.
    pub status: RebaseStatus,
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
    /// One entry per promoted repo, in name order.
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
            writeln!(
                w,
                "  {}  {}",
                repo.repo,
                match repo.status {
                    RebaseStatus::Rebased => "rebased",
                    RebaseStatus::Skipped => "skipped",
                    RebaseStatus::Conflicted => "conflicted",
                }
            )?;
        }
        Ok(())
    }
}

/// Rebase every promoted repo of `input.name` onto its default branch.
///
/// Blocked when the feature does not exist. Per-repo problems are warnings on
/// a clean report — skipped (dirty, or no worktree) and conflicted repos
/// continue the batch, exactly like `deliver`'s best-effort pushes.
pub fn rebase(ctx: &Ctx, input: RebaseInput) -> Outcome<RebaseOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let name = FeatureName::new(input.name)?;
    let mut feature = Feature::read(&layout, &name)?.ok_or_else(|| {
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

    // A feature-wide rebase touches every promoted repo, so it preflights the
    // whole set first: no mutation happens if any target is locked. A
    // successful receipt pins its promotion individually (and the whole child
    // once closed `integrated`); a failed-evidence receipt does not lock its
    // repo.
    for repo in feature.promotions.keys() {
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

    for (repo_name, promotion) in &feature.promotions {
        let worktree = layout.repo_worktree(repo_name, &feature.branch);
        let manifest_repo = manifest
            .repos()
            .iter()
            .find(|repo| repo.name() == repo_name);

        let Some(manifest_repo) = manifest_repo else {
            repos.push(RepoRebase {
                repo: repo_name.clone(),
                status: RebaseStatus::Skipped,
            });
            warnings.push(Warning::new(
                "rebase.repo_not_in_manifest",
                repo_name.as_str(),
                "not in ivar.json; nothing to rebase onto",
            ));
            continue;
        };
        let target = onto
            .clone()
            .unwrap_or_else(|| base::resolve(&feature, promotion, manifest_repo.default_branch()));

        if !fs::is_dir(&worktree)? {
            repos.push(RepoRebase {
                repo: repo_name.clone(),
                status: RebaseStatus::Skipped,
            });
            warnings.push(Warning::new(
                "rebase.no_worktree",
                repo_name.as_str(),
                "no worktree materialised for this repo",
            ));
            continue;
        }

        // Rebase over uncommitted work is how it gets lost — a dirty worktree
        // is skipped, never rebased around.
        if git.worktree_dirty(&worktree)? {
            repos.push(RepoRebase {
                repo: repo_name.clone(),
                status: RebaseStatus::Skipped,
            });
            warnings.push(Warning::new(
                "rebase.dirty",
                repo_name.as_str(),
                "worktree has uncommitted changes; commit or stash them first",
            ));
            continue;
        }

        match git.rebase_branch(&worktree, target.as_str()) {
            Ok(()) => {
                repos.push(RepoRebase {
                    repo: repo_name.clone(),
                    status: RebaseStatus::Rebased,
                });
                if onto.is_some() {
                    collapsed.push(repo_name.clone());
                }
            }
            Err(git::Error::Refused { .. }) => {
                // The rebase stopped — a conflict, most likely. Abort it so
                // the worktree is exactly where it was, then move on.
                if let Err(abort) = git.abort_rebase(&worktree) {
                    warnings.push(Warning::new(
                        "rebase.abort_failed",
                        repo_name.as_str(),
                        format!("could not abort the stopped rebase: {abort}"),
                    ));
                }
                repos.push(RepoRebase {
                    repo: repo_name.clone(),
                    status: RebaseStatus::Conflicted,
                });
                warnings.push(Warning::new(
                    "rebase.conflicted",
                    repo_name.as_str(),
                    "rebase stopped (likely a conflict) and was aborted",
                ));
            }
            Err(other) => return Err(other.into()),
        }
    }
    repos.sort_by(|a, b| a.repo.cmp(&b.repo));

    // Collapse the base only where the worktree actually landed on it — the
    // declaration and the worktree move together, or neither moves.
    if let Some(onto) = &onto
        && !collapsed.is_empty()
    {
        for repo_name in &collapsed {
            if let Some(promotion) = feature.promotions.get_mut(repo_name) {
                promotion.base = Some(onto.clone());
            }
        }
        feature.write(&layout)?;
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

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/rebase.rs"]
mod tests;
