//! Unit tests for `crate::store::feature`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::feature::{
    ApprovalState, ExecutionBoard, ExecutionGraph, ExecutionStatus, Gate, GateState, JournalEntry,
    WorkstreamDef, WorkstreamStatus,
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
        text.contains("\"version\": 3"),
        "the store must stamp the version: {text}"
    );
    assert!(text.contains("\"branch\": \"feat/checkout\""));
}

// -- v1 -> v3 migration (base field, then nested-integration fields) -------

#[test]
fn a_v1_feature_json_migrates_on_read_and_persists_as_v3() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();

    // Hand-crafted v1 feature.json — the shape ivar actually wrote before
    // the v2 bump: no `base` field anywhere.
    let path = layout.feature_dir(&name).join("feature.json");
    fs::ensure_dir(path.parent().unwrap()).unwrap();
    fs::write_text(
        &path,
        r#"{"version":1,"name":"checkout","branch":"feat/checkout","promotions":{"api":{"worktree":"pending"}}}"#,
    )
    .unwrap();

    let migrated = Feature::read(&layout, &name)
        .unwrap()
        .expect("migration must succeed");

    // The v1→v2 step is a version stamp only; the v2→v3 step fills the
    // nested-integration fields with their empty shapes.
    assert_eq!(migrated.version(), 3);
    assert_eq!(migrated.base, None);
    assert_eq!(migrated.parent, None);
    assert_eq!(migrated.integration, crate::domain::feature::IntegrationOverride::default());
    assert_eq!(
        migrated
            .promotions
            .get(&RepoName::new("api").unwrap())
            .unwrap()
            .base,
        None
    );
    assert_eq!(
        migrated
            .promotions
            .get(&RepoName::new("api").unwrap())
            .unwrap()
            .integration_receipt,
        None
    );

    let text = fs::read_text(&path).unwrap().unwrap();
    assert!(
        text.contains("\"version\": 3"),
        "disk must hold v3 after migration: {text}"
    );
}

#[test]
fn a_v2_feature_json_migrates_on_read_and_persists_as_v3() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();

    // Hand-crafted v2 feature.json — the shape ivar actually wrote before
    // the v3 bump: `base` present, but no `parent`, no `integration`
    // override, and no `integration_receipt` on any promotion.
    let path = layout.feature_dir(&name).join("feature.json");
    fs::ensure_dir(path.parent().unwrap()).unwrap();
    fs::write_text(
        &path,
        r#"{"version":2,"name":"checkout","branch":"feat/checkout","promotions":{"api":{"worktree":"ready","base":"main"}},"base":null}"#,
    )
    .unwrap();

    let migrated = Feature::read(&layout, &name)
        .unwrap()
        .expect("migration must succeed");

    assert_eq!(migrated.version(), 3);
    assert_eq!(migrated.parent, None);
    assert_eq!(
        migrated.integration,
        crate::domain::feature::IntegrationOverride::default()
    );
    let api = RepoName::new("api").unwrap();
    assert_eq!(migrated.promotions[&api].integration_receipt, None);
    // v2 data is preserved through the step.
    assert_eq!(
        migrated.promotions[&api].worktree,
        crate::domain::feature::WorktreeState::Ready
    );
    assert_eq!(
        migrated.promotions[&api].base,
        Some(BranchName::new("main").unwrap())
    );

    let text = fs::read_text(&path).unwrap().unwrap();
    assert!(
        text.contains("\"version\": 3"),
        "disk must hold v3 after migration: {text}"
    );
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
            write_contract: vec!["src/".to_owned()],
            status: WorkstreamStatus::Waiting,
            provider: None,
            model: None,
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

    // Write a hand-crafted v1 board.json — the shape the ivar actually
    // wrote before the v2 bump: `status`, `graph {workstreams,
    // plan_fingerprint}`, `journal`, no seq/event_id, no provider/agent.
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
                    "write_contract": ["src/"],
                    "status": "active"
                },
                {
                    "id": "ws2",
                    "title": "WS two",
                    "operations": ["op2"],
                    "depends_on": ["ws1"],
                    "write_contract": ["tests/"],
                    "status": "waiting"
                }
            ],
            "plan_fingerprint": "v1-fp"
        },
        "journal": [
            {
                "kind": "prepared",
                "timestamp": "1700000000",
                "workstream": "board",
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

    // Existing status and workstream statuses are preserved.
    assert_eq!(migrated.status, ExecutionStatus::Running);
    assert_eq!(
        migrated.graph.workstreams[0].status,
        WorkstreamStatus::Active
    );
    assert_eq!(
        migrated.graph.workstreams[1].status,
        WorkstreamStatus::Waiting
    );

    // New fields get sensible defaults.
    assert_eq!(
        migrated.next_event_seq, 2,
        "one journal entry → next seq is 2"
    );
    assert!(migrated.blocked_by.is_none());
    assert!(migrated.sessions.is_empty());

    // Journal entries gained seq and event_id.
    let entry = &migrated.journal[0];
    assert_eq!(entry.seq, 1);
    assert!(!entry.event_id.is_empty());

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
