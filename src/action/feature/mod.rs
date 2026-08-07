//! `ivar feature` — manage features and the repos promoted into them.
//!
//! A **Feature** is one branch across the repos it has **Promoted**. This
//! module owns the lifecycle: `create` (name + branch, nothing promoted),
//! `list` (what exists), `promote` (materialise a repo's worktree on the
//! feature branch), `demote` (remove a repo from the feature), and `status`
//! (per-feature detail).
//!
//! The worktree creation on promote is the heart of the slice — see
//! [`promote`] for the branch-from-default-branch rule.

pub mod create;
pub mod demote;
pub mod list;
pub mod promote;
pub mod status;
