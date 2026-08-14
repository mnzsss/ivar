//! `ivar feature` — manage features and the repos promoted into them.
//!
//! A **Feature** is one branch across the repos it has **Promoted**. This
//! module owns the lifecycle: `create` (name + branch, nothing promoted),
//! `list` (what exists), `promote` (materialise a repo's worktree on the
//! feature branch), `demote` (remove a repo from the feature), `status`
//! (per-feature detail), `deliver` (push a feature's branches), `close`
//! (record the outcome and stop the feature's sessions), `delete` (tear the
//! feature down), `rebase` (bring promoted worktrees up to date), `review`
//! (open the feature in VSCode), and `prune` (delete features whose branches
//! are fully merged, never one with a live session).
//!
//! The worktree creation on promote is the heart of the slice — see
//! [`promote`] for the branch-from-default-branch rule.

mod base;
pub mod close;
pub mod create;
pub mod delete;
pub mod deliver;
pub mod demote;
pub mod list;
pub mod promote;
pub mod prune;
pub mod rebase;
pub mod review;
pub mod status;
pub mod view;
