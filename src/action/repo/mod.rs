//! `ivar repo` — manage the repos a hall knows about.
//!
//! Subcommands: `list`, `add`, `remove`, `pull`, `setup`, `upstream`. Each is
//! one file, one function of the `fn verb(ctx, input) -> Outcome<Outcome_>`
//! shape — see ARCHITECTURE.md, "1. `action` is the unit, and it has one
//! output shape".
//!
//! The line this module draws: it manages the **bare clone and the manifest
//! entry**. Worktrees for branches other than the default are owned by
//! `ivar feature` (slice 4) — a repo command that created arbitrary
//! worktrees would be a second, undocumented answer to "which branch is
//! materialised where".

pub mod add;
pub mod create;
pub mod list;
pub mod pull;
pub mod remove;
pub mod setup;
pub mod upstream;

use crate::domain::name::RepoName;
use crate::error::{Failure, FixAction};
use crate::store::manifest::Manifest;

/// Blocks when `name` is already declared in the manifest — the collision
/// `repo add` and `repo create` both refuse before writing anything.
pub(super) fn ensure_name_free(manifest: &Manifest, name: &RepoName) -> Result<(), Failure> {
    for existing in manifest.repos() {
        if existing.name() == name {
            return Err(Failure::blocked(
                "repo.name_exists",
                format!("`{name}` is already in ivar.json"),
            )
            .expected("a repo name not already declared")
            .actual(format!("`{name}` is already declared"))
            .fix(FixAction::safe(
                "repo.remove_first",
                format!("Remove `{name}` first with `ivar repo remove {name}`, then add again."),
            )));
        }
    }
    Ok(())
}
