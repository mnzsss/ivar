//! `ivar` mounts the repos a feature spans into one directory — real git
//! worktrees on the same branch, opened by one agent session.
//!
//! # The model
//!
//! A **Hall** owns N **Repos** as bare clones. A **Feature** is one branch across
//! the repos it has **Promoted**. A **Session** materialises a **View Dir** of
//! symlinks into exactly those worktrees and opens a harness in it. Repos the
//! feature has not promoted are held read-only by the kernel.
//!
//! Two properties constrain every module here.
//!
//! **The work does not live inside a vendor.** Hall, feature, branch, promoted
//! repos, view dir and plan are files on disk and commits in git. A session dying
//! loses the conversation and nothing else. So no state may exist only in a
//! running process, and no verb may require a live session to be useful.
//!
//! **Read-only is a filesystem guarantee, not a harness one.** Non-promoted
//! worktrees have their write bits cleared. Harness hooks are the *error message*
//! that names the way out, never the barrier.
//!
//! # Layering
//!
//! Dependencies point downward only. `tests/architecture.rs` enforces this — a
//! convention nobody remembers is not a boundary.
//!
//! | module    | may import                     | may **not** import                            |
//! |-----------|--------------------------------|-----------------------------------------------|
//! | `cli`     | `action`, `error`              | everything else                               |
//! | `action`  | anything below                 | `cli`                                         |
//! | `domain`  | `error`                        | `store`, `git`, `harness`, `tui`, `infra`     |
//! | `store`   | `domain`, `infra`, `error`     | `action`, `git`, `harness`, `tui`             |
//! | `git`     | `infra`, `error`               | `action`, `store`, `domain`, `harness`, `tui` |
//! | `harness` | `domain`, `infra`, `error`     | `action`, `store`, `git`                      |
//! | `tui`     | `domain`, `infra`, `error`     | `action`, `store`, `git`, `harness`           |
//! | `infra`   | `error`                        | everything else                               |
//!
//! `domain` is pure so its invariants are testable without a temp directory.
//! `tui` cannot reach `action` or `store` because state is pushed *into* the
//! driver by the host loop — that is what makes the widget a referentially
//! transparent projection.
//!
//! See `ARCHITECTURE.md` for the full module map and the build order.

// Module tree, declared as each vertical slice lands. Order is deliberate:
// see the build order in ARCHITECTURE.md.
//
pub mod error;
pub mod infra;

pub mod domain;
pub mod store;

pub mod git;
pub mod harness;
pub mod providers;
// `action` and `cli` are not part of the published API. They are `pub` only so
// `src/bin/ivar.rs` and the integration tests, which are separate crates, can
// reach them.
//
// Without `#[doc(hidden)]`, cargo-semver-checks counts all 225 items in these two
// modules as public. Adding one field to any `*Input`/`*Outcome` struct then
// registers as a breaking change, which fills the release-plz PR with false
// positives and forces a minor bump for an internal refactor.
#[doc(hidden)]
pub mod action;
#[doc(hidden)]
pub mod cli;
pub mod tui;

#[cfg(test)]
#[path = "../tests/support/unit.rs"]
mod test_support;

// Landing with their slices:
// pub mod tui;

/// The binary and crate name, used wherever a message names the tool.
pub const NAME: &str = "ivar";
