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
//! another feature, now delivered — has landed. It rewrites `Promotion::base`
//! to `<branch>` for **every** promoted repo before anything is rebased, then
//! rebases each worktree onto that same new target: the declaration and the
//! worktree move together. Without it, each repo rebases onto its own
//! individually-resolved base, same as today.
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
    /// Collapse the base: rewrite every promoted repo's declared base to
    /// this branch, unvalidated, before rebasing anything onto it.
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

    // `--onto` collapses the base: every promoted repo's declared base
    // becomes the new target, recorded before any worktree is touched — the
    // same "statement about the future" `promote` itself makes. The rebase
    // loop below then needs no special case for it: it always rebases onto
    // each repo's effective base, which this has just made `onto` for all of
    // them.
    if let Some(onto) = input.onto {
        let onto = BranchName::new(onto)?;
        for promotion in feature.promotions.values_mut() {
            promotion.base = Some(onto.clone());
        }
        feature.write(&layout)?;
    }

    let mut repos = Vec::new();
    let mut warnings = Vec::new();
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
        let target = base::resolve(&feature, promotion, manifest_repo.default_branch());

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
