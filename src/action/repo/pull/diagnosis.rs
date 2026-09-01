//! Whether a diverged default branch can be moved, and what to say when it
//! cannot.
//!
//! Split from the command surface because it answers a different question.
//! `mod.rs` sequences a run over every repo; this file decides, for one repo,
//! whether `--resolve` may reset a branch and which of the two blockers a
//! human is actually looking at.

use std::collections::HashSet;

use camino::Utf8Path;

use crate::git::{Divergence, Git};
use crate::store::manifest::Repo;

/// The remote-tracking ref `fetch_branch` keeps current — the reset target for
/// a safely-resolved divergence.
pub(super) fn remote_ref(repo: &Repo) -> String {
    format!("origin/{}", repo.default_branch())
}

/// Why `--resolve` would not reset a branch.
///
/// Carried rather than collapsed to a bool because the two blockers have
/// different recoveries: uncommitted work is parked or inspected by hand,
/// while genuine divergence is read with `--diagnose` first. Reporting both as
/// "cannot fast-forward" leaves a user guessing which one they have.
pub(super) enum ResetBlocker {
    /// Uncommitted work in the default worktree.
    Dirty,
    /// Local commits that are not duplicates of anything upstream — or a check
    /// that could not be completed, which is treated the same way on purpose.
    Diverged,
}

impl ResetBlocker {
    /// The sentence a human gets, naming the blocker and one safe next step.
    ///
    /// Every suggestion here is read-only or reversible. Nothing offers to
    /// reset, delete, stash, or commit on the user's behalf: the whole reason
    /// this path was reached is that ivar could not prove what is safe to
    /// discard.
    pub(super) fn reason(&self) -> &'static str {
        match self {
            Self::Dirty => {
                "the default branch has diverged from the remote and the worktree has \
                 uncommitted changes; inspect them with `git status`, and park them with \
                 `git stash` if they are not needed"
            }
            Self::Diverged => {
                "the default branch has diverged from the remote and the local commits are \
                 not duplicates of upstream work; inspect them with `ivar repo pull --diagnose`"
            }
        }
    }
}

/// Whether resetting the default branch to the remote tip is safe: every
/// local-only commit is a duplicate of work already in the remote, and the
/// worktree is clean (so nothing uncommitted is discarded).
///
/// `Ok(())` means safe. Conservative by construction: any failure to confirm a
/// duplicate — a patch-id that cannot be read, a dirty worktree, a missing ref
/// — is a blocker, and the branch is left for the human. `--resolve` never
/// touches a branch it cannot prove is a duplicate.
pub(super) fn safe_to_reset(
    git: &impl Git,
    worktree: &Utf8Path,
    repo: &Repo,
) -> Result<(), ResetBlocker> {
    let branch = repo.default_branch().as_str();
    let remote = remote_ref(repo);

    // A dirty worktree must not be reset — the reset would discard work that
    // was never committed and so never reached the remote. `unwrap_or(true)`:
    // a worktree whose state cannot be read is assumed to hold work.
    if git.worktree_dirty(worktree).unwrap_or(true) {
        return Err(ResetBlocker::Dirty);
    }

    let Ok(divergence) = git.divergence(worktree, branch, &remote) else {
        return Err(ResetBlocker::Diverged);
    };
    if divergence.local_only.is_empty() {
        // Nothing local to lose would mean it did not diverge; be safe anyway.
        return Err(ResetBlocker::Diverged);
    }

    let Some(remote_patch_ids) = divergence
        .remote_only
        .iter()
        .map(|commit| git.commit_patch_id(worktree, &commit.sha).ok())
        .collect::<Option<HashSet<String>>>()
    else {
        return Err(ResetBlocker::Diverged);
    };

    // Squash case: the whole local range, as one cumulative diff, matches a
    // single remote commit — local commits re-landed as a squash.
    let contained_as_squash = git
        .merge_base(worktree, branch, &remote)
        .ok()
        .and_then(|base| git.diff_patch_id(worktree, &base, branch).ok())
        .map(|local_cumulative| remote_patch_ids.contains(&local_cumulative))
        .unwrap_or(false);

    // Rebase / cherry-pick case: every local-only commit individually matches
    // a remote commit.
    let contained_per_commit = divergence.local_only.iter().all(|commit| {
        git.commit_patch_id(worktree, &commit.sha)
            .map(|id| remote_patch_ids.contains(&id))
            .unwrap_or(false)
    });

    if contained_as_squash || contained_per_commit {
        Ok(())
    } else {
        Err(ResetBlocker::Diverged)
    }
}

/// The local-vs-remote commit lists behind a "cannot fast-forward" report —
/// the `--diagnose` view.
///
/// Reads the worktree's checked-out branch against its remote-tracking
/// counterpart (`origin/<branch>`, the ref `fetch_branch` updates). Best-effort:
/// if the refs cannot be read the diagnosis is `None`, so the pull's own
/// "skipped" status still stands — a diagnosis failure must not turn a skipped
/// repo into a failed one.
pub(super) fn diagnose_divergence(
    git: &impl Git,
    worktree: &Utf8Path,
    repo: &Repo,
) -> Option<Divergence> {
    let branch = repo.default_branch().as_str();
    git.divergence(worktree, branch, &format!("origin/{branch}"))
        .ok()
}
