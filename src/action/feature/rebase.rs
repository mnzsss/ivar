//! `ivar feature rebase <name>` — rebase every promoted repo's feature-branch
//! worktree onto its default branch.
//!
//! The point of a rebase here is to bring a feature's work up to date with the
//! work that landed on the default branches since the feature branched. Each
//! promoted repo's worktree (on the feature branch) is replayed on top of that
//! repo's `default_branch` from `ivar.json`.
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
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git};
use crate::infra::fs;

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// What `ivar feature rebase` needs.
#[derive(Debug, Clone)]
pub struct RebaseInput {
    /// The feature's name.
    pub name: String,
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

    let mut repos = Vec::new();
    let mut warnings = Vec::new();
    for repo_name in feature.promotions.keys() {
        let worktree = layout.repo_worktree(repo_name, &feature.branch);
        let default_branch = manifest
            .repos()
            .iter()
            .find(|repo| repo.name() == repo_name)
            .map(|repo| repo.default_branch().to_string());

        let Some(default_branch) = default_branch else {
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

        match git.rebase_branch(&worktree, &default_branch) {
            Ok(()) => repos.push(RepoRebase {
                repo: repo_name.clone(),
                status: RebaseStatus::Rebased,
            }),
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
