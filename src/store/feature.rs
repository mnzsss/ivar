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

/// Migrate a board.json from v1 → v2.
///
/// v1 carried a flat `status` on the board and nested everything inside
/// `graph`.  v2 flattens workstreams to the top level, adds per-workstream
/// statuses, drops the wrapper object, and introduces new fields with
/// sensible defaults.
fn v1_to_v2(mut value: serde_json::Value) -> Result<serde_json::Value, String> {
    let root = value
        .as_object_mut()
        .ok_or("board must be an object")?;

    // --- workstreams -------------------------------------------------------
    let graph = match root.remove("graph") {
        Some(serde_json::Value::Object(g)) => g,
        _ => return Err("missing graph".to_owned()),
    };

    let mut workstreams = Vec::new();
    if let Some(streams) = graph.get("workstreams").and_then(|w| w.as_array()) {
        for ws in streams {
            let obj = ws.as_object().ok_or("workstream not an object")?;

            // write_contract may be {"paths": [...]} (v1) or [...] (already v2);
            // always normalise to a bare array.
            let wc = match obj.get("write_contract") {
                Some(serde_json::Value::Object(inner)) => inner
                    .get("paths")
                    .and_then(|p| p.as_array())
                    .cloned()
                    .unwrap_or_default(),
                Some(arr) if arr.is_array() => arr
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
                _ => Vec::new(),
            };

            let depends = obj
                .get("depends_on")
                .and_then(|d| d.as_array())
                .map(|arr| arr.len() > 0)
                .unwrap_or(false);

            let status = if depends { "waiting" } else { "active" };

            let entry = serde_json::json!({
                "id": obj.get("id").cloned().unwrap_or_default(),
                "title": obj.get("title").cloned().unwrap_or_default(),
                "operations": obj.get("operations").cloned().unwrap_or_else(|| serde_json::json!([])),
                "depends_on": obj.get("depends_on").cloned().unwrap_or_else(|| serde_json::json!([])),
                "write_contract": wc,
                "status": status,
                "provider": null,
                "agent": null,
            });
            workstreams.push(entry);
        }
    }

    // --- journal -----------------------------------------------------------
    let journal = root
        .remove("journal")
        .and_then(|j| {
            if j.is_array() {
                Some(j.as_array().cloned().unwrap_or_default())
            } else {
                None
            }
        })
        .unwrap_or_default();

    // --- build v2 shape ----------------------------------------------------
    let v2 = serde_json::json!({
        "version": BOARD_VERSION,
        "status": "pending",
        "workstreams": workstreams,
        "plan_fingerprint": root.get("plan_fingerprint").cloned().unwrap_or_default(),
        "journal": journal,
        "next_event_seq": 1,
        "blocked_by": null,
        "sessions": {},
    });

    Ok(v2)
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
        vec![Migration::new(1, 2, v1_to_v2)],
        BOARD_VERSION,
        Policy::Local,
    )
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::domain::feature::{
        ApprovalState, ExecutionBoard, ExecutionGraph, ExecutionStatus, Gate, GateState,
        JournalEntry, WorkstreamDef, WorkstreamStatus,
    };
    use crate::domain::name::{BranchName, FeatureName, RepoName};
    use crate::infra::fs;
    use crate::test_support::hall_root;

    fn layout_with_hall() -> (tempfile::TempDir, Layout) {
        let (guard, root) = hall_root();
        // A feature directory needs no ivar.json to exist, but Layout paths
        // are computed from the root alone; write a manifest so the directory
        // is a real (if empty) hall.
        let _ = root.join("ivar.json");
        (guard, Layout::at(root))
    }

    #[test]
    fn absent_feature_reads_as_ok_none() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();

        assert_eq!(Feature::read(&layout, &name).unwrap(), None);
    }

    #[test]
    fn write_then_read_round_trips() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();
        let mut feature = Feature::new(name.clone(), BranchName::new("feat/checkout").unwrap());
        feature.promote(RepoName::new("api").unwrap());

        feature.write(&layout).unwrap();
        let read_back = Feature::read(&layout, &name).unwrap().unwrap();

        assert_eq!(read_back, feature);
        assert!(read_back.is_promoted(&RepoName::new("api").unwrap()));
    }

    #[test]
    fn the_file_is_written_under_the_features_directory() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();
        let feature = Feature::new(name.clone(), BranchName::new("feat/checkout").unwrap());

        feature.write(&layout).unwrap();

        assert!(fs::is_file(&layout.feature_dir(&name).join("feature.json")).unwrap());
    }

    #[test]
    fn a_file_newer_than_the_binary_is_refused() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();
        let path = layout.feature_dir(&name).join("feature.json");
        fs::ensure_dir(path.parent().unwrap()).unwrap();
        fs::write_text(
            &path,
            r#"{"version":99,"name":"checkout","branch":"feat/checkout","promotions":{}}"#,
        )
        .unwrap();

        let error = Feature::read(&layout, &name).unwrap_err();

        assert_eq!(error.code, "store.version_too_new");
    }

    #[test]
    fn the_written_shape_is_canonical_and_version_stamped() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();
        let feature = Feature::new(name.clone(), BranchName::new("feat/checkout").unwrap());

        feature.write(&layout).unwrap();

        let text = fs::read_text(&layout.feature_dir(&name).join("feature.json"))
            .unwrap()
            .unwrap();
        assert!(
            text.contains("\"version\": 1"),
            "the store must stamp the version: {text}"
        );
        assert!(text.contains("\"branch\": \"feat/checkout\""));
    }

    // -- approvals ------------------------------------------------------------

    #[test]
    fn absent_approvals_read_as_ok_none() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();

        assert_eq!(ApprovalState::read(&layout, &name).unwrap(), None);
    }

    #[test]
    fn approvals_write_then_read_round_trips() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();
        let mut approvals = ApprovalState::fresh();
        approvals.set(
            Gate::Requirements,
            GateState::Approved,
            Some("fp".to_owned()),
        );

        approvals.write(&layout, &name).unwrap();
        let read_back = ApprovalState::read(&layout, &name).unwrap().unwrap();

        assert_eq!(read_back, approvals);
        assert_eq!(
            read_back.state(Gate::Requirements),
            Some(GateState::Approved)
        );
    }

    #[test]
    fn approvals_land_in_the_planning_directory() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();
        let approvals = ApprovalState::fresh();

        approvals.write(&layout, &name).unwrap();

        assert!(fs::is_file(&layout.planning_dir(&name).join("approvals.json")).unwrap());
    }

    #[test]
    fn approvals_are_canonical_and_version_stamped() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();
        let mut approvals = ApprovalState::fresh();
        approvals.set(
            Gate::Requirements,
            GateState::Approved,
            Some("fp".to_owned()),
        );

        approvals.write(&layout, &name).unwrap();

        let text = fs::read_text(&layout.planning_dir(&name).join("approvals.json"))
            .unwrap()
            .unwrap();
        assert!(
            text.contains("\"version\": 1"),
            "the store must stamp the version: {text}"
        );
        assert!(text.contains("\"gate\": \"requirements\""));
        assert!(text.contains("\"state\": \"approved\""));
    }

    // -- execution board -------------------------------------------------------

    fn execution_board() -> ExecutionBoard {
        ExecutionBoard::new(ExecutionGraph {
            plan_fingerprint: "abc123".to_owned(),
            workstreams: vec![WorkstreamDef {
                id: "ws1".to_owned(),
                title: "WS one".to_owned(),
                operations: vec!["op1".to_owned()],
                depends_on: Vec::new(),
                write_contract: vec!["src/".to_owned()].into(),
                status: WorkstreamStatus::Waiting,
                provider: None,
                agent: None,
            }],
        })
    }

    #[test]
    fn absent_board_reads_as_ok_none() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();

        assert_eq!(ExecutionBoard::read(&layout, &name).unwrap(), None);
    }

    #[test]
    fn board_write_then_read_round_trips() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();
        let mut board = execution_board();
        board.set_status(ExecutionStatus::Running);
        board.push_journal(JournalEntry::new("board", "prepared", "board prepared"));

        board.write(&layout, &name).unwrap();
        let read_back = ExecutionBoard::read(&layout, &name).unwrap().unwrap();

        assert_eq!(read_back, board);
        assert_eq!(read_back.status, ExecutionStatus::Running);
        assert_eq!(read_back.journal.len(), 1);
    }

    #[test]
    fn the_board_lands_in_the_execution_directory() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();
        let board = execution_board();

        board.write(&layout, &name).unwrap();

        assert!(fs::is_file(&layout.execution_dir(&name).join("board.json")).unwrap());
    }

    #[test]
    fn the_board_is_canonical_and_version_stamped() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();
        let board = execution_board();

        board.write(&layout, &name).unwrap();

        let text = fs::read_text(&layout.execution_dir(&name).join("board.json"))
            .unwrap()
            .unwrap();
        assert!(
            text.contains("\"version\": 2"),
            "the store must stamp the version: {text}"
        );
        assert!(text.contains("\"status\": \"pending\""));
        assert!(text.contains("\"plan_fingerprint\": \"abc123\""));
    }

    // -- v1 → v2 migration ---------------------------------------------------

    #[test]
    fn a_v1_board_migrates_on_read_and_persists_as_v2() {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();

        // Write a hand-crafted v1 board.json — flat `status`, nested `graph`,
        // write_contract as {"paths": [...]}, no new fields.
        let v1_board = serde_json::json!({
            "version": 1,
            "status": "running",
            "graph": {
                "workstreams": [
                    {
                        "id": "ws1",
                        "title": "WS one",
                        "operations": ["op1"],
                        "depends_on": [],
                        "write_contract": {"paths": ["src/"]},
                        "status": "active"
                    },
                    {
                        "id": "ws2",
                        "title": "WS two",
                        "operations": ["op2"],
                        "depends_on": ["ws1"],
                        "write_contract": {"paths": ["tests/"]},
                        "status": "waiting"
                    }
                ],
                "plan_fingerprint": "v1-fp"
            },
            "journal": [
                {
                    "kind": "prepared",
                    "timestamp": "1700000000",
                    "message": "board prepared"
                }
            ]
        });

        fs::ensure_dir(&layout.execution_dir(&name)).unwrap();
        fs::write_text(
            &layout.execution_dir(&name).join("board.json"),
            &serde_json::to_string(&v1_board).unwrap(),
        )
        .unwrap();

        // Read through the store — triggers migration under Policy::Local.
        let migrated = ExecutionBoard::read(&layout, &name)
            .unwrap()
            .expect("migration must succeed");

        // Core structure preserved.
        assert_eq!(migrated.graph.workstreams.len(), 2);
        assert_eq!(migrated.graph.plan_fingerprint, "v1-fp");
        assert_eq!(migrated.journal.len(), 1);

        // New fields get sensible defaults.
        assert_eq!(migrated.status, ExecutionStatus::Pending);
        assert_eq!(migrated.next_event_seq, 1);
        assert!(migrated.blocked_by.is_none());
        assert!(migrated.sessions.is_empty());

        // Workstream statuses computed from depends_on.
        assert_eq!(
            migrated.graph.workstreams[0].status,
            WorkstreamStatus::Active
        );
        assert_eq!(
            migrated.graph.workstreams[1].status,
            WorkstreamStatus::Waiting
        );

        // provider / agent default to None (hall default path).
        for ws in &migrated.graph.workstreams {
            assert!(ws.provider.is_none());
            assert!(ws.agent.is_none());
        }

        // File persisted as v2 on disk.
        let text = fs::read_text(&layout.execution_dir(&name).join("board.json"))
            .unwrap()
            .unwrap();
        assert!(
            text.contains("\"version\": 2"),
            "disk must hold v2 after migration: {text}"
        );
    }
}
