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
use crate::domain::provider::Provider;
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
    assert_eq!(
        migrated.integration,
        crate::domain::feature::IntegrationOverride::default()
    );
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
        text.contains("\"version\": 3"),
        "the store must stamp the version: {text}"
    );
    assert!(text.contains("\"status\": \"pending\""));
    assert!(text.contains("\"plan_fingerprint\": \"abc123\""));
}

// -- v1 → v3 migration ---------------------------------------------------
// A v1 board walks the whole contiguous chain v1 → v2 → v3: the v2 step
// numbers the journal and fills provider/agent, then the v3 step gives every
// entry an explicit `revision: null`.

#[test]
fn a_v1_board_migrates_on_read_and_persists_as_v3() {
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

    // Journal entries gained seq and event_id, and the revision is unknown.
    let entry = &migrated.journal[0];
    assert_eq!(entry.seq, 1);
    assert!(!entry.event_id.is_empty());
    assert_eq!(entry.revision, None, "legacy entries carry no revision");

    // provider / agent default to None (hall default path).
    for ws in &migrated.graph.workstreams {
        assert!(ws.provider.is_none());
        assert!(ws.agent.is_none());
    }

    // File persisted as v3 on disk.
    let text = fs::read_text(&layout.execution_dir(&name).join("board.json"))
        .unwrap()
        .unwrap();
    assert!(
        text.contains("\"version\": 3"),
        "disk must hold v3 after migration: {text}"
    );
}

// -- v2 → v3 migration (revision on journal entries) ----------------------

/// A representative v2 board — the shape ivar actually wrote before the v3
/// bump: seq/event_id present on every journal entry, no `revision` field
/// anywhere — migrates on local read with every existing field and the
/// entry order unchanged, gains an explicit `revision: null` on each entry,
/// and persists as v3.
#[test]
fn a_v2_board_migrates_on_read_and_persists_as_v3() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();

    let v2_board = serde_json::json!({
        "version": 2,
        "status": "running",
        "graph": {
            "workstreams": [
                {
                    "id": "ws1",
                    "title": "WS one",
                    "operations": ["op1"],
                    "depends_on": [],
                    "write_contract": ["src/"],
                    "status": "active",
                    "provider": null,
                    "agent": null
                }
            ],
            "plan_fingerprint": "v2-fp"
        },
        "journal": [
            {
                "seq": 1,
                "event_id": "prepared.1",
                "timestamp": "1700000000",
                "workstream": "board",
                "kind": "prepared",
                "message": "board prepared"
            },
            {
                "seq": 2,
                "event_id": "started.2",
                "timestamp": "1700000001",
                "workstream": "ws1",
                "kind": "started",
                "message": "started ws1"
            },
            {
                "seq": 3,
                "event_id": "produced.3",
                "timestamp": "1700000002",
                "workstream": "ws1",
                "kind": "produced",
                "message": "changed things"
            }
        ],
        "next_event_seq": 4,
        "blocked_by": null,
        "sessions": {}
    });

    fs::ensure_dir(&layout.execution_dir(&name)).unwrap();
    fs::write_text(
        &layout.execution_dir(&name).join("board.json"),
        &serde_json::to_string(&v2_board).unwrap(),
    )
    .unwrap();

    let migrated = ExecutionBoard::read(&layout, &name)
        .unwrap()
        .expect("v2 board must migrate");

    // The v3 step fills `revision: null` and leaves everything else alone.
    assert_eq!(migrated.version, 3);
    assert_eq!(migrated.status, ExecutionStatus::Running);
    assert_eq!(migrated.graph.plan_fingerprint, "v2-fp");
    assert_eq!(migrated.next_event_seq, 4);
    assert!(migrated.blocked_by.is_none());
    assert!(migrated.sessions.is_empty());

    // Every journal entry survived, in order, with its fields intact and an
    // explicit null revision — legacy evidence with unknown revision.
    let expected: Vec<(u64, &str, &str, &str)> = vec![
        (1, "prepared.1", "board", "prepared"),
        (2, "started.2", "ws1", "started"),
        (3, "produced.3", "ws1", "produced"),
    ];
    assert_eq!(migrated.journal.len(), expected.len());
    for (entry, (seq, event_id, workstream, kind)) in migrated.journal.iter().zip(expected.iter()) {
        assert_eq!(entry.seq, *seq);
        assert_eq!(entry.event_id.as_str(), *event_id);
        assert_eq!(entry.workstream.as_str(), *workstream);
        assert_eq!(entry.kind.as_str(), *kind);
        assert_eq!(entry.revision, None, "migration adds a null revision");
    }

    // Persisted as v3 on disk.
    let text = fs::read_text(&layout.execution_dir(&name).join("board.json"))
        .unwrap()
        .unwrap();
    assert!(
        text.contains("\"version\": 3"),
        "disk must hold v3 after migration: {text}"
    );
    assert!(
        text.contains("\"revision\": null"),
        "the migrated journal entry must carry an explicit null revision: {text}"
    );
}

/// A v2 board's migration is lossless in the strictest sense a migration can
/// be: the raw JSON's non-version fields and the journal's order survive
/// byte-for-byte once the version stamp and the added `revision: null` are
/// set aside. This pins that the v2→v3 step rewrites nothing it did not add.
#[test]
fn the_v2_to_v3_migration_rewrites_no_existing_field() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();

    let v2_board = serde_json::json!({
        "version": 2,
        "status": "blocked",
        "graph": {
            "workstreams": [
                {
                    "id": "ws1",
                    "title": "WS one",
                    "operations": ["op1", "op2"],
                    "depends_on": ["ws0"],
                    "write_contract": ["src/a/", "src/b/"],
                    "status": "paused",
                    "provider": "claude-code",
                    "agent": "implementer-x"
                }
            ],
            "plan_fingerprint": "fp-9"
        },
        "journal": [
            {
                "seq": 5,
                "event_id": "question.asked.2.5",
                "timestamp": "1700000005",
                "workstream": "ws1",
                "kind": "question.asked",
                "message": "which way?"
            }
        ],
        "next_event_seq": 6,
        "blocked_by": "ws1",
        "sessions": { "sess-1": "ws1" }
    });
    let original = v2_board.to_string();

    fs::ensure_dir(&layout.execution_dir(&name)).unwrap();
    fs::write_text(&layout.execution_dir(&name).join("board.json"), &original).unwrap();

    let migrated = ExecutionBoard::read(&layout, &name)
        .unwrap()
        .expect("v2 board must migrate");

    // Fields the migration neither added nor stamped are byte-identical.
    assert_eq!(migrated.status, ExecutionStatus::Blocked);
    assert_eq!(migrated.graph.plan_fingerprint, "fp-9");
    assert_eq!(migrated.graph.workstreams[0].operations, vec!["op1", "op2"]);
    assert_eq!(migrated.graph.workstreams[0].depends_on, vec!["ws0"]);
    assert_eq!(
        migrated.graph.workstreams[0].write_contract,
        vec!["src/a/", "src/b/"]
    );
    assert_eq!(
        migrated.graph.workstreams[0].status,
        WorkstreamStatus::Paused
    );
    assert_eq!(
        migrated.graph.workstreams[0].provider,
        Some(Provider::ClaudeCode)
    );
    assert_eq!(
        migrated.graph.workstreams[0].agent.as_deref(),
        Some("implementer-x")
    );
    assert_eq!(migrated.next_event_seq, 6);
    assert_eq!(migrated.blocked_by.as_deref(), Some("ws1"));
    assert_eq!(
        migrated.sessions.get("sess-1").map(String::as_str),
        Some("ws1")
    );
    assert_eq!(migrated.journal.len(), 1);
    assert_eq!(migrated.journal[0].seq, 5);
    assert_eq!(migrated.journal[0].kind, "question.asked");
    assert_eq!(migrated.journal[0].revision, None);

    // The only journal-level change is the added revision: the entry's other
    // fields match what was written.
    assert_eq!(migrated.journal[0].message, "which way?");
    assert_eq!(migrated.journal[0].workstream, "ws1");
    assert_eq!(migrated.journal[0].timestamp, "1700000005");
    assert_eq!(migrated.journal[0].event_id, "question.asked.2.5");
}
