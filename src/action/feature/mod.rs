//! `ivar feature` — manage features and the repos promoted into them.
//!
//! A **Feature** is one branch across the repos it has **Promoted**. This
//! module owns the lifecycle: `create` (name + branch, nothing promoted),
//! `list` (what exists), `promote` (materialise a repo's worktree on the
//! feature branch), `demote` (remove a repo from the feature), `status`
//! (per-feature detail), `deliver` (push a feature's branches), `close`
//! (record the outcome and stop the feature's sessions), `delete` (tear the
//! feature down), `rebase` (bring promoted worktrees up to date), and `prune`
//! (delete features whose branches are fully merged, never one with a live
//! session).
//!
//! The worktree creation on promote is the heart of the slice — see
//! [`promote`] for the branch-from-default-branch rule.

mod base;
pub mod cleanup;
pub mod close;
pub mod create;
pub mod delete;
pub mod deliver;
pub mod demote;
pub mod integrate;
mod lifecycle;
pub mod list;
mod mutation;
mod parent_promotion;
pub mod promote;
pub mod prune;
mod pull_requests;
pub mod rebase;
mod relations;
pub mod rename;
pub mod reparent;
pub mod status;
mod tree;
mod verification;
pub mod view;
pub mod workspace;

// The scoped mutation guards are exposed `pub(crate)` so plan and session
// mutations can enforce partial-integration boundaries without reaching into
// this module's internals. See `mutation` for the scopes.
pub(crate) use mutation::{ensure_not_fully_integrated, ensure_unrestricted_session_allowed};
pub(crate) use relations::descendants;

pub mod select;

use crate::domain::name::FeatureName;
use crate::error::Warning;
use crate::store::layout::Layout;

/// Drops a feature's graph layers; a failure is a warning, never a stop,
/// because the feature directory is already gone and `ivar graph index`
/// reconciles leftovers.
pub(super) fn drop_layers_or_warn(layout: &Layout, name: &FeatureName) -> Option<Warning> {
    let db_path = layout.ivar_dir().join("memory.db");
    if !db_path.is_file() {
        return None;
    }
    let dropped = crate::action::graph::lock_index(layout)
        .map_err(|failure| failure.what)
        .and_then(|_lock| {
            crate::store::graph::db::GraphDb::open(db_path.as_std_path())
                .map_err(|e| e.to_string())
                .and_then(|db| {
                    db.drop_feature_layers(name.as_str())
                        .map_err(|e| e.to_string())
                })
        });
    dropped.err().map(|reason| {
        Warning::new(
            "graph.layers_not_dropped",
            name.to_string(),
            format!("graph layers were not dropped: {reason}; run `ivar graph index`"),
        )
    })
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/lifecycle_graph.rs"]
mod lifecycle_graph_tests;
