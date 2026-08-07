//! `ivar repo` — manage the repos a hall knows about.
//!
//! Subcommands: `list`, `add`, `remove`, `pull`. Each is one file, one
//! function of the `fn verb(ctx, input) -> Outcome<Outcome_>` shape —
//! see ARCHITECTURE.md, "1. `action` is the unit, and it has one output
//! shape".
//!
//! The line this module draws: it manages the **bare clone and the manifest
//! entry**. Worktrees for branches other than the default are owned by
//! `ivar feature` (slice 4) — a repo command that created arbitrary
//! worktrees would be a second, undocumented answer to "which branch is
//! materialised where".

pub mod add;
pub mod list;
pub mod pull;
pub mod remove;
