//! Feature state on disk: `features/<name>/feature.json`.
//!
//! One file per feature, under the feature's directory, written through the
//! versioned [`Store`] with [`Policy::Local`] — this is derived local state
//! (a teammate's clone has no reason to carry another team's feature
//! worktrees), so a read that migrates persists the migrated form, silently.
//!
//! `Feature` itself carries its `version` field, the same way `Manifest`
//! does; the store stamps it on write and the type declares it, so a
//! hand-edited file with a newer version is refused rather than adopted.
//!
//! The feature's **approval state** lives alongside it at
//! `features/<name>/planning/approvals.json`, through the same versioned
//! store. `ApprovalState` deliberately carries no `version` field of its own —
//! the store stamps the schema version onto the JSON value, and the type
//! accepts it as an unknown field.
//!
//! The feature's **execution board** lives at
//! `features/<name>/execution/board.json`, also through the versioned store.
//! `ExecutionBoard` carries its own `version` field, like `Feature` does.

use camino::Utf8PathBuf;

use crate::domain::feature::{ApprovalState, ExecutionBoard, Feature};
use crate::domain::name::FeatureName;
use crate::error::Failure;
use crate::store::layout::Layout;
use crate::store::versioned::{Migration, Policy, Store};

/// `feature.json`'s schema version. Matches [`Feature`]'s own constant —
/// the type owns the number, this module just wires it into the store.
const CURRENT_VERSION: u32 = 1;

/// `approvals.json`'s schema version.
const APPROVALS_VERSION: u32 = 1;

/// `board.json`'s schema version.
const BOARD_VERSION: u32 = 2;

/// The filename every feature's promotion record lives in, under its
/// feature directory. One file, not one-per-repo: promotions are a small
/// map and rewriting one file is atomic through the canonical writer.
const FEATURE_FILE: &str = "feature.json";

/// The filename each feature's approval state lives in, under its planning
/// directory.
const APPROVALS_FILE: &str = "approvals.json";

/// The filename each feature's execution board lives in, under its execution
/// directory.
const BOARD_FILE: &str = "board.json";

impl Feature {
    /// Read `features/<name>/feature.json`. `Ok(None)` when the feature has
    /// never been written — a feature created but never promoted.
    ///
    /// A file newer than this binary understands is a hard error; see
    /// [`Store::read`].
    pub fn read(layout: &Layout, name: &FeatureName) -> Result<Option<Self>, Failure> {
        store(layout, name).read().map_err(Failure::from)
    }

    /// Write this feature to `features/<name>/feature.json`, atomically, in
    /// canonical form. Creates the feature directory if it does not exist —
    /// `feature create` calls this on a brand-new feature.
    pub fn write(&self, layout: &Layout) -> Result<(), Failure> {
        let dir = layout.feature_dir(&self.name);
        crate::infra::fs::ensure_dir(&dir)?;
        store(layout, &self.name).write(self).map_err(Failure::from)
    }
}

impl ApprovalState {
    /// Read `features/<name>/planning/approvals.json`. `Ok(None)` when no
    /// gate has ever been approved or invalidated.
    ///
    /// A file newer than this binary understands is a hard error; see
    /// [`Store::read`].
    pub fn read(layout: &Layout, name: &FeatureName) -> Result<Option<Self>, Failure> {
        approvals_store(layout, name).read().map_err(Failure::from)
    }

    /// Write this approval state to
    /// `features/<name>/planning/approvals.json`, atomically, in canonical
    /// form. Creates the planning directory if it does not exist.
    pub fn write(&self, layout: &Layout, name: &FeatureName) -> Result<(), Failure> {
        crate::infra::fs::ensure_dir(&layout.planning_dir(name))?;
        approvals_store(layout, name)
            .write(self)
            .map_err(Failure::from)
    }
}

impl ExecutionBoard {
    /// Read `features/<name>/execution/board.json`. `Ok(None)` when no board
    /// has ever been prepared for the feature.
    ///
    /// A file newer than this binary understands is a hard error; see
    /// [`Store::read`].
    pub fn read(layout: &Layout, name: &FeatureName) -> Result<Option<Self>, Failure> {
        board_store(layout, name).read().map_err(Failure::from)
    }

    /// Write this board to `features/<name>/execution/board.json`, atomically,
    /// in canonical form. Creates the execution directory if it does not
    /// exist — `feature execute prepare` calls this on a brand-new board.
    pub fn write(&self, layout: &Layout, name: &FeatureName) -> Result<(), Failure> {
        crate::infra::fs::ensure_dir(&layout.execution_dir(name))?;
        board_store(layout, name).write(self).map_err(Failure::from)
    }
}

/// The path of a feature's execution board file. Public because the action
/// layer names it in its outcome and its failure messages; the filename
/// itself stays this module's to own, so no path arithmetic leaks into
/// `action`.
#[must_use]
pub fn board_path(layout: &Layout, name: &FeatureName) -> Utf8PathBuf {
    layout.execution_dir(name).join(BOARD_FILE)
}

/// Migrate a board.json from v0 → v1. The board has never had a v0 shape:
/// it has been written with `version: 1` since the day it shipped, like
/// `ivar.json` itself. The step exists to keep the chain contiguous — a file
/// with no `version` field at all is treated as v1 and passed through; the
/// store stamps the final version regardless.
fn v0_to_v1(value: serde_json::Value) -> Result<serde_json::Value, String> {
    Ok(value)
}

/// Migrate a board.json from v1 → v2.
///
/// v2 keeps v1's shape — `status`, `graph {workstreams, plan_fingerprint}`,
/// `journal` — and adds fields with sensible defaults: `next_event_seq` and
/// `seq`/`event_id` on the journal (the monotonic order and identity that
/// make tick/reply idempotent), `blocked_by` and `sessions` on the board, and
/// `provider`/`agent` on each workstream (the executor override that lets a
/// workstream declare where it runs).
fn v1_to_v2(mut value: serde_json::Value) -> Result<serde_json::Value, String> {
    let root = value.as_object_mut().ok_or("board must be an object")?;

    // --- workstreams: add provider/agent where missing ----------------------
    let graph = root
        .get_mut("graph")
        .and_then(|g| g.as_object_mut())
        .ok_or("board is missing graph")?;
    if let Some(streams) = graph.get_mut("workstreams").and_then(|w| w.as_array_mut()) {
        for ws in streams {
            let obj = ws.as_object_mut().ok_or("workstream not an object")?;
            obj.entry("provider").or_insert(serde_json::Value::Null);
            obj.entry("agent").or_insert(serde_json::Value::Null);
        }
    }

    // --- journal: number the entries with seq/event_id ----------------------
    let mut fallback = Vec::new();
    let journal = root
        .get_mut("journal")
        .and_then(|j| j.as_array_mut())
        .unwrap_or(&mut fallback);
    for (index, entry) in journal.iter_mut().enumerate() {
        let obj = entry.as_object_mut().ok_or("journal entry not an object")?;
        let seq = (index + 1) as u64;
        let kind = obj
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| "unknown".to_owned());
        obj.entry("seq").or_insert(serde_json::Value::from(seq));
        obj.entry("event_id")
            .or_insert_with(|| serde_json::Value::String(format!("migrated.v1.{kind}.{seq}")));
    }
    let next_seq = (journal.len() + 1) as u64;

    // --- board: new fields with sensible defaults ---------------------------
    root.entry("next_event_seq")
        .or_insert(serde_json::Value::from(next_seq));
    root.entry("blocked_by").or_insert(serde_json::Value::Null);
    root.entry("sessions")
        .or_insert(serde_json::Value::Object(Default::default()));

    Ok(value)
}

/// The versioned store over one feature's file.
fn store(layout: &Layout, name: &FeatureName) -> Store<Feature> {
    Store::new(
        layout.feature_dir(name).join(FEATURE_FILE),
        Vec::new(),
        CURRENT_VERSION,
        Policy::Local,
    )
}

/// The versioned store over one feature's approvals file.
fn approvals_store(layout: &Layout, name: &FeatureName) -> Store<ApprovalState> {
    Store::new(
        layout.planning_dir(name).join(APPROVALS_FILE),
        Vec::new(),
        APPROVALS_VERSION,
        Policy::Local,
    )
}

/// The versioned store over one feature's execution board file.
fn board_store(layout: &Layout, name: &FeatureName) -> Store<ExecutionBoard> {
    Store::new(
        board_path(layout, name),
        vec![
            Migration::new(0, 1, v0_to_v1),
            Migration::new(1, 2, v1_to_v2),
        ],
        BOARD_VERSION,
        Policy::Local,
    )
}

#[cfg(test)]
#[path = "../../tests/unit/store/feature.rs"]
mod tests;
