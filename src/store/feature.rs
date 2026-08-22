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
//! The feature's **Run Receipt** lives at `features/<name>/execution/run.json`,
//! with its archive alongside it. That whole surface is [`run`]'s, including
//! the private legacy-board import that turns a pre-receipt
//! `execution/board.json` into a terminal imported receipt.
//!
//! The feature's **execution board** still lives at
//! `features/<name>/execution/board.json`, also through the versioned store.
//! `ExecutionBoard` carries its own `version` field, like `Feature` does. It is
//! retained only until the board actions are deleted; nothing new should reach
//! for it, and the legacy importer reads old boards through its own private DTOs
//! rather than through this type. The board's filename and its v0 → v3
//! migration chain live in [`run::legacy`], not here, so both outlive the store
//! below — N-COMPAT is permanent, and the store is not.

pub mod run;

use camino::Utf8PathBuf;

use crate::domain::feature::{ApprovalState, ExecutionBoard, Feature};
use crate::domain::name::FeatureName;
use crate::error::Failure;
use crate::store::layout::Layout;
use crate::store::versioned::{Migration, Policy, Store};

/// `feature.json`'s schema version. Matches [`Feature`]'s own constant —
/// the type owns the number, this module just wires it into the store.
const CURRENT_VERSION: u32 = 3;

/// `approvals.json`'s schema version. v2 dropped the fourth `execution_graph`
/// gate record; see [`approvals_v1_to_v2`].
const APPROVALS_VERSION: u32 = 2;

/// `board.json`'s schema version.
const BOARD_VERSION: u32 = 3;

/// The filename every feature's promotion record lives in, under its
/// feature directory. One file, not one-per-repo: promotions are a small
/// map and rewriting one file is atomic through the canonical writer.
const FEATURE_FILE: &str = "feature.json";

/// The filename each feature's approval state lives in, under its planning
/// directory.
const APPROVALS_FILE: &str = "approvals.json";

/// The serde name of the gate that v2 retired. A string rather than a `Gate`
/// variant because the whole point is that the variant is gone.
const RETIRED_GATE: &str = "execution_graph";

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
    run::legacy::board_path(layout, name)
}

/// Migrate a feature.json from v0 → v1. Like `board.json`'s own `v0_to_v1`
/// step (now in [`run::legacy`]), `feature.json` has shipped with `version: 1`
/// stamped since it first existed — this step exists only to keep the chain contiguous from
/// 0, which [`Store::new`] requires once any migration is registered.
fn feature_v0_to_v1(value: serde_json::Value) -> Result<serde_json::Value, String> {
    Ok(value)
}

/// Migrate a feature.json from v1 → v2. v2 adds `base: Option<BranchName>`
/// to `Feature` and `Promotion`, both `#[serde(default)]` — a v1 file has no
/// `base` field and deserialises it as `None` without this step's help. The
/// step is a version stamp only, filling nothing: `store` cannot import
/// `action`, and the feature's effective base (the declared branch, or the
/// repo's `default_branch`) is computed from the manifest, which this
/// module has no access to.
fn feature_v1_to_v2(value: serde_json::Value) -> Result<serde_json::Value, String> {
    Ok(value)
}

/// Migrate a feature.json from v2 → v3. v3 adds the nested-integration
/// fields: `parent: Option<FeatureName>` and `integration: IntegrationOverride`
/// on `Feature`, and `integration_receipt: Option<IntegrationReceipt>` under
/// each promotion. All three are `#[serde(default)]`, so a v2 file would
/// deserialise without this step's help; the step still fills the explicit
/// empty shapes so the persisted v3 is the canonical form (and so a later
/// tool reading the file sees the fields it expects, not their absence).
fn feature_v2_to_v3(mut value: serde_json::Value) -> Result<serde_json::Value, String> {
    let root = value.as_object_mut().ok_or("feature must be an object")?;
    root.entry("parent").or_insert(serde_json::Value::Null);
    root.entry("integration").or_insert(serde_json::json!({}));
    let promotions = root
        .get_mut("promotions")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("feature is missing promotions")?;
    for promotion in promotions.values_mut() {
        promotion
            .as_object_mut()
            .ok_or("promotion must be an object")?
            .entry("integration_receipt")
            .or_insert(serde_json::Value::Null);
    }
    Ok(value)
}

/// The versioned store over one feature's file.
fn store(layout: &Layout, name: &FeatureName) -> Store<Feature> {
    Store::new(
        layout.feature_dir(name).join(FEATURE_FILE),
        vec![
            Migration::new(0, 1, feature_v0_to_v1),
            Migration::new(1, 2, feature_v1_to_v2),
            Migration::new(2, 3, feature_v2_to_v3),
        ],
        CURRENT_VERSION,
        Policy::Local,
    )
}

/// Migrate an approvals.json from v1 → v2.
///
/// v2 is the three-gate lifecycle. A v1 file carries a fourth record for the
/// `execution_graph` gate, and that gate no longer exists as a `Gate`
/// variant — so the record has to go *here*, at the JSON-value layer, before
/// `ApprovalState`'s derived `Deserialize` ever sees it. Left in place it
/// would not read as an unknown gate to be normalized away; it would fail the
/// whole file to parse, and a user's approvals would be unreadable rather
/// than merely one gate shorter.
///
/// Everything else is untouched: the first three records keep their order,
/// their states, and their fingerprints. What ivar knew a human had approved
/// does not change because a fourth gate stopped existing.
fn approvals_v1_to_v2(mut value: serde_json::Value) -> Result<serde_json::Value, String> {
    let root = value
        .as_object_mut()
        .ok_or("approval state must be an object")?;
    let gates = root
        .get_mut("gates")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or("approval state is missing gates")?;
    gates.retain(|record| {
        record.get("gate").and_then(serde_json::Value::as_str) != Some(RETIRED_GATE)
    });
    Ok(value)
}

/// The versioned store over one feature's approvals file.
///
/// The chain starts at v1, not v0: `approvals.json` has been written with
/// `version: 1` stamped since it first existed, so there is no unversioned
/// predecessor to migrate from and a v0 file is refused as unreachable rather
/// than adopted.
fn approvals_store(layout: &Layout, name: &FeatureName) -> Store<ApprovalState> {
    Store::new(
        layout.planning_dir(name).join(APPROVALS_FILE),
        vec![Migration::new(1, 2, approvals_v1_to_v2)],
        APPROVALS_VERSION,
        Policy::Local,
    )
}

/// The versioned store over one feature's execution board file.
fn board_store(layout: &Layout, name: &FeatureName) -> Store<ExecutionBoard> {
    Store::new(
        board_path(layout, name),
        vec![
            Migration::new(0, 1, run::legacy::v0_to_v1),
            Migration::new(1, 2, run::legacy::v1_to_v2),
            Migration::new(2, 3, run::legacy::v2_to_v3),
        ],
        BOARD_VERSION,
        Policy::Local,
    )
}

#[cfg(test)]
#[path = "../../tests/unit/store/feature.rs"]
mod tests;
