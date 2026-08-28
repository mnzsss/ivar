//! `ivar feature rename <feature> [--name <name>] [--branch <branch>]` —
//! rename a feature, its branch, or both.
//!
//! Rename is the identity transition: it can change `Feature.name`,
//! `Feature.branch`, or both, and every consequence of that change — each
//! promoted repo's local branch and worktree path, its remote branch when
//! published, direct children's `parent`, live sessions, and `plans/<name>/`
//! — moves together, or none of it does.
//!
//! # Preflight, then mutate — never the other way around
//!
//! Every local and remote precondition (source/target collisions, dependent
//! feature/session readability, per-repo worktree/branch state, delivery
//! remote reachability, open PRs) is checked and aggregated into one
//! [`Failure::blocked`] before anything is written. See [`preflight`].
//!
//! # A durable, resumable, reversible transition
//!
//! Once preflight passes, a [`Transition`] marker is written under the
//! *source* feature's directory (`.ivar/features/<old-name>/.renaming`) —
//! chosen over the destination, unlike [`super::super::session::conversion`]'s
//! marker, because the destination directory does not exist yet at this
//! point and the source directory is guaranteed to exist until the very last
//! forward step. The marker moves with the feature directory when the name
//! changes, so a resumed rename looks for it at both the source and
//! destination `.ivar/features/` paths.
//!
//! Every step is idempotent, both forward and in reverse: each checks
//! whatever postcondition it is trying to reach and only does the missing
//! work, so a crash at any point — mid-step, mid-rollback — is safe to retry
//! by running `ivar feature rename` again with the same arguments. A
//! mutation failure walks the completed steps backward, undoing each with
//! the same disk-state-derived idempotency, and only removes the marker once
//! the source state (on rollback) or the target state (on success) is fully
//! restored. If an inverse cannot complete safely — most commonly, a remote
//! ref changed out from under it — the marker is kept and the failure names
//! the exact divergence and the manual recovery required.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, WriteHuman};
use crate::git;

use super::super::{discover_hall, read_manifest};
use super::relations;

mod plan;
mod steps;

/// What `ivar feature rename` needs.
#[derive(Debug, Clone)]
pub struct RenameInput {
    /// The feature to rename.
    pub feature: String,
    /// The new name, unvalidated. `None` leaves the name unchanged.
    pub name: Option<String>,
    /// The new branch, unvalidated. `None` leaves the branch unchanged.
    pub branch: Option<String>,
}

/// One promoted repo's rename outcome.
#[derive(Debug, Clone, Serialize)]
pub struct RepoRenameOutcome {
    /// The repo that was renamed.
    pub repo: RepoName,
    /// Whether its local branch was renamed. `false` when only the feature
    /// name changed.
    pub branch_renamed: bool,
    /// Whether its worktree moved to a new path. `false` when only the
    /// feature name changed.
    pub worktree_moved: bool,
    /// The remote outcome, when the repo has a configured delivery remote and
    /// the old branch was published there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

/// What `ivar feature rename` did.
#[derive(Debug, Clone, Serialize)]
pub struct RenameOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature's identity before the rename.
    pub old_name: FeatureName,
    /// The feature's identity after the rename.
    pub new_name: FeatureName,
    /// The branch before the rename.
    pub old_branch: BranchName,
    /// The branch after the rename.
    pub new_branch: BranchName,
    /// One entry per promoted repo, in name order.
    pub repos: Vec<RepoRenameOutcome>,
}

impl WriteHuman for RenameOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.old_name == self.new_name {
            writeln!(
                w,
                "Renamed feature `{}`'s branch from `{}` to `{}` in {}",
                self.old_name, self.old_branch, self.new_branch, self.root
            )?;
        } else if self.old_branch == self.new_branch {
            writeln!(
                w,
                "Renamed feature `{}` to `{}` in {}",
                self.old_name, self.new_name, self.root
            )?;
        } else {
            writeln!(
                w,
                "Renamed feature `{}` to `{}` (branch `{}` to `{}`) in {}",
                self.old_name, self.new_name, self.old_branch, self.new_branch, self.root
            )?;
        }
        for repo in &self.repos {
            let mut moved = Vec::new();
            if repo.branch_renamed {
                moved.push("branch");
            }
            if repo.worktree_moved {
                moved.push("worktree");
            }
            if !moved.is_empty() {
                writeln!(w, "  {}: {} renamed", repo.repo, moved.join(" and "))?;
            }
            if let Some(remote) = &repo.remote {
                writeln!(w, "  {}: {remote}", repo.repo)?;
            }
        }
        Ok(())
    }
}

/// Rename `input.feature`'s name, branch, or both.
///
/// Refuses — before any mutation — when neither target value actually
/// differs from the current one, when the source feature or any dependent
/// (child, session) cannot be read, when a target name/branch/path collides
/// with something else, or when a promoted repo's remote state cannot be
/// established or has an open PR against the old branch. See [`preflight`]
/// for the whole gate.
///
/// If an interrupted rename's marker is found for `input.feature` (at either
/// its old or new identity), it is resumed regardless of what `input` asks
/// for — the marker wins, exactly as `session convert`'s does.
pub fn rename(ctx: &Ctx, input: RenameInput) -> Outcome<RenameOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let feature_name = FeatureName::new(input.feature)?;

    // An interrupted rename for this feature wins over the request — the
    // marker may live at the old or the new identity depending on how far a
    // previous attempt got, so both are checked before ordinary validation
    // runs (which would otherwise misjudge a feature already mid-move).
    if let Some((anchor, transition)) = steps::find_transition(&layout, &feature_name)? {
        return steps::resume(&layout, &manifest, &git, &anchor, transition);
    }

    let renamed_name = input
        .name
        .map(FeatureName::new)
        .transpose()?
        .unwrap_or_else(|| feature_name.clone());
    let renamed_branch_input = input.branch.map(BranchName::new).transpose()?;

    let source = relations::read_feature(&layout, &feature_name)?;
    let renamed_branch = renamed_branch_input
        .clone()
        .unwrap_or_else(|| source.branch.clone());

    if renamed_name == source.name && renamed_branch == source.branch {
        return Err(Failure::blocked(
            "feature.rename_noop",
            format!("`{feature_name}` already has that name and branch"),
        )
        .expected("a new name and/or a new branch")
        .actual("both supplied values equal the feature's current ones")
        .fix(FixAction::safe(
            "feature.rename_change_something",
            "Pass `--name` and/or `--branch` with a value that actually differs.",
        )));
    }

    let plan = plan::build(
        &layout,
        &manifest,
        &git,
        &source,
        renamed_name,
        renamed_branch_input,
    )?;

    if !plan.1.is_empty() {
        return Err(Failure::blocked(
            "rename.blocked",
            "Rename blocked by preflight checks".to_owned(),
        )
        .details(serde_json::to_value(&plan.1).unwrap_or(serde_json::Value::Null)));
    }

    steps::run(&layout, &manifest, &git, plan.0)
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/feature/rename.rs"]
mod tests;
