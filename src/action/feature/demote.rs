//! `ivar feature demote <feature> <repo>` — remove a repo from a feature.
//!
//! Demoting removes the promotion record. The worktree stays on disk unless
//! `--remove-worktree` is passed — removing work can destroy uncommitted work,
//! so that flag removes the worktree and its local branch only when nothing
//! would be lost, and otherwise refuses before touching the record.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::Feature;
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git};
use crate::infra::fs;

use super::super::{discover_hall, read_manifest};
use super::base;
use crate::action::Ctx;
use crate::store::layout::Layout;

/// What `ivar feature demote` needs.
#[derive(Debug, Clone)]
pub struct DemoteInput {
    /// The feature's name.
    pub feature: String,
    /// The repo to demote.
    pub repo: String,
    /// Also remove the repo's worktree and local branch, when that loses nothing.
    pub remove_worktree: bool,
}

/// What `ivar feature demote` did.
#[derive(Debug, Clone, Serialize)]
pub struct DemoteOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the repo was demoted from.
    pub feature: FeatureName,
    /// The repo that was demoted.
    pub repo: RepoName,
    /// Whether the repo's worktree was removed from disk.
    pub worktree_removed: bool,
    /// Whether the repo's local feature branch was deleted.
    pub branch_deleted: bool,
}

impl WriteHuman for DemoteOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.worktree_removed {
            let branch = if self.branch_deleted {
                " and its local branch"
            } else {
                ""
            };
            writeln!(
                w,
                "Demoted `{}` from feature `{}`. Removed its worktree{branch}.",
                self.repo, self.feature,
            )
        } else {
            writeln!(
                w,
                "Demoted `{}` from feature `{}`. Its worktree stays on disk — `ivar cleanup` can remove it, or demote with `--remove-worktree`.",
                self.repo, self.feature,
            )
        }
    }
}

/// Demote `input.repo` from `input.feature`.
///
/// Blocked when the feature does not exist or the repo was never promoted —
/// both name the way out, and neither leaves a half-edited record. With
/// `remove_worktree`, also blocked while removing would lose work; that check
/// runs before the record changes.
pub fn demote(ctx: &Ctx, input: DemoteInput) -> Outcome<DemoteOutcome> {
    let layout = discover_hall(ctx)?;
    let feature_name = FeatureName::new(input.feature)?;
    let repo_name = RepoName::new(input.repo)?;
    let git = git::System;

    let removal_target = if input.remove_worktree {
        Feature::read(&layout, &feature_name)?
            .filter(|feature| feature.is_promoted(&repo_name))
            .map(|feature| ensure_removable(&layout, &git, &feature, &repo_name).map(|()| feature))
            .transpose()?
    } else {
        None
    };

    Feature::update(&layout, &feature_name, |feature| {
        // Removing a repo is a membership change — frozen once this promotion
        // carries a successful receipt, and by the whole-child `integrated`
        // close.
        super::mutation::ensure_promotion_mutable(&layout, feature, &repo_name)?;

        if feature.demote(&repo_name) {
            Ok(())
        } else {
            Err(Failure::blocked(
                "feature.not_promoted",
                format!("`{repo_name}` is not promoted into `{feature_name}`"),
            )
            .expected("a repo currently promoted into this feature")
            .actual("this repo has no promotion record here")
            .fix(FixAction::safe(
                "feature.promote_first",
                format!("Run `ivar feature promote {feature_name} {repo_name}` first."),
            )))
        }
    })?;

    let (worktree_removed, branch_deleted, warnings) = match &removal_target {
        Some(feature) => remove_worktree_and_branch(&layout, &git, feature, &repo_name),
        None => (false, false, Vec::new()),
    };

    Ok(Report::with_warnings(
        DemoteOutcome {
            root: layout.root().to_path_buf(),
            feature: feature_name,
            repo: repo_name,
            worktree_removed,
            branch_deleted,
        },
        warnings,
    ))
}

/// Refuses unless dropping `repo`'s worktree and branch loses nothing: a clean
/// tree, no commits beyond the base, and a branch absent from the remote.
fn ensure_removable(
    layout: &Layout,
    git: &impl Git,
    feature: &Feature,
    repo: &RepoName,
) -> Result<(), Failure> {
    let manifest = read_manifest(layout)?;
    let Some(manifest_repo) = manifest.repos().iter().find(|r| r.name() == repo) else {
        return Err(Failure::blocked(
            "feature.demote_repo_unknown",
            format!("cannot remove the worktree of `{repo}`: it is not in ivar.json"),
        )
        .expected("a repo declared in ivar.json, so its base and remote are known")
        .actual("the repo is not declared"));
    };
    let Some(promotion) = feature.promotions.get(repo) else {
        return Ok(());
    };
    let bare = layout.repo_bare(repo);
    let branch = feature.branch.as_str();
    let mut blockers = Vec::new();

    match git::lookup_worktree(git, &bare, branch) {
        Ok(Some(worktree)) if !worktree.prunable => match git.worktree_dirty(&worktree.path) {
            Ok(true) => {
                blockers.push("the worktree has uncommitted or untracked changes".to_owned())
            }
            Ok(false) => {}
            Err(error) => blockers.push(format!("cannot read the worktree: {error}")),
        },
        Ok(_) => {}
        Err(error) => blockers.push(format!("cannot look up the worktree: {error}")),
    }

    if git.revision_commit(&bare, branch).is_ok() {
        let base = base::resolve(feature, promotion, manifest_repo.default_branch());
        match base::unmerged_commits(git, &bare, base.as_str(), branch) {
            Ok(0) => {}
            Ok(ahead) => blockers.push(format!("the branch has {ahead} commit(s) beyond `{base}`")),
            Err(error) => {
                blockers.push(format!("cannot compare the branch with `{base}`: {error}"))
            }
        }
        match git.remote_branch_tip(&bare, manifest_repo.url(), branch) {
            Ok(None) => {}
            Ok(Some(_)) => blockers.push("the branch is published on the remote".to_owned()),
            Err(error) => blockers.push(format!("cannot check the remote: {error}")),
        }
    }

    if blockers.is_empty() {
        return Ok(());
    }
    Err(Failure::blocked(
        "feature.demote_worktree_not_removable",
        format!("cannot remove the worktree of `{repo}`: removing it would lose work"),
    )
    .expected("a clean worktree, no commits beyond the base, and an unpublished branch")
    .actual(blockers.join("; "))
    .fix(FixAction::safe(
        "feature.demote_keep_worktree",
        format!(
            "Resolve the blockers, or run `ivar feature demote {} {repo}` without `--remove-worktree`.",
            feature.name
        ),
    )))
}

/// Best-effort, after the record is gone: a failure becomes a warning and the
/// leftover is still reachable through `ivar cleanup`.
fn remove_worktree_and_branch(
    layout: &Layout,
    git: &impl Git,
    feature: &Feature,
    repo: &RepoName,
) -> (bool, bool, Vec<Warning>) {
    let bare = layout.repo_bare(repo);
    let branch = feature.branch.as_str();
    let mut warnings = Vec::new();

    let worktree_removed = match git::lookup_worktree(git, &bare, branch) {
        Ok(Some(entry)) => match git::remove_worktree_entry(git, &bare, &entry) {
            Ok(()) => {
                fs::prune_empty_parents(&entry.path, &layout.repo_dir(repo));
                true
            }
            Err(error) => {
                warnings.push(Warning::new(
                    "feature.demote_worktree_failed",
                    repo.as_str(),
                    error.to_string(),
                ));
                false
            }
        },
        Ok(None) => true,
        Err(error) => {
            warnings.push(Warning::new(
                "feature.demote_worktree_failed",
                repo.as_str(),
                error.to_string(),
            ));
            false
        }
    };

    let branch_deleted = worktree_removed
        && git.revision_commit(&bare, branch).is_ok()
        && match git.delete_branch(&bare, branch) {
            Ok(()) => true,
            Err(error) => {
                warnings.push(Warning::new(
                    "feature.demote_branch_failed",
                    repo.as_str(),
                    error.to_string(),
                ));
                false
            }
        };

    (worktree_removed, branch_deleted, warnings)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/demote.rs"]
mod tests;
