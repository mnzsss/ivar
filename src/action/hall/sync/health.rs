//! Whether the hall's sync is stuck, read from git on the spot.
//!
//! Nothing is recorded anywhere: every caller asks git again, so a hall the
//! user fixed by hand stops being reported the moment it is fixed. No network
//! is touched, so ahead and behind are as fresh as the last fetch.

use crate::git::{self, Git, Operation};
use crate::store::layout::Layout;

use super::sync_state;

/// What the hall's repository says about its sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncHealth {
    /// Nothing to report: not a git repository, no upstream, in step with it,
    /// or only behind it (a pull will fix that, and nothing is lost).
    Healthy,
    /// A merge, rebase or cherry-pick stopped and waits to be finished.
    InProgress(Operation),
    /// Both sides have commits the other lacks; the rebase to join them
    /// conflicted, or has not been tried.
    Diverged {
        /// The local branch.
        branch: String,
        /// The upstream, as a human writes it (`origin/main`).
        upstream: String,
        /// Commits only the local branch has.
        ahead: usize,
        /// Commits only the upstream has.
        behind: usize,
    },
    /// Local commits the upstream does not have yet, and nothing newer there.
    Unpushed {
        /// The local branch.
        branch: String,
        /// The upstream, as a human writes it (`origin/main`).
        upstream: String,
        /// Commits that have not been pushed.
        ahead: usize,
    },
}

impl SyncHealth {
    /// Whether the sync is stopped until someone acts: an operation half-done,
    /// or a divergence. An unpushed commit is not blocked, only waiting.
    #[must_use]
    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::InProgress(_) | Self::Diverged { .. })
    }
}

/// The sync health of the hall at `layout`. A repository that cannot be read
/// is [`SyncHealth::Healthy`]: this answers "is the sync stuck?", and a hall
/// that is unreadable is for the other checks to name.
#[must_use]
pub fn sync_health(layout: &Layout, git: &impl Git) -> SyncHealth {
    let Ok(state) = sync_state(layout, git) else {
        return SyncHealth::Healthy;
    };
    if !state.is_repository {
        return SyncHealth::Healthy;
    }
    if let Some(operation) = state.operation {
        return SyncHealth::InProgress(operation);
    }
    let (Some(branch), Some(upstream)) = (state.branch.clone(), state.upstream.as_ref()) else {
        return SyncHealth::Healthy;
    };
    let upstream = upstream.name.clone();
    if state.diverged() {
        SyncHealth::Diverged {
            branch,
            upstream,
            ahead: state.ahead,
            behind: state.behind,
        }
    } else if state.ahead > 0 {
        SyncHealth::Unpushed {
            branch,
            upstream,
            ahead: state.ahead,
        }
    } else {
        SyncHealth::Healthy
    }
}

/// [`sync_health`] of the hall at `layout`, with the system's git.
#[must_use]
pub fn system_sync_health(layout: &Layout) -> SyncHealth {
    sync_health(layout, &git::System)
}
