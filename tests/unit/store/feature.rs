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
use crate::domain::feature::{
    CheckStatus, CoordinatorReport, RunBaseline, RunDiff, RunId, RunOutcome, RunProvenance,
    RunReceipt, RunStatus, TaskResult, TaskStatus, VerificationCheck,
};
use crate::domain::name::{BranchName, FeatureName, RepoName, SessionId};
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
        text.contains("\"version\": 2"),
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

// -- approvals v1 → v2 (the retired execution-graph gate) -------------------

/// The migration a real hall hits: an `approvals.json` written when there were
/// four gates. The first three keep their states and their fingerprints —
/// what ivar knew a human had approved does not change because a fourth gate
/// stopped existing — and only the `execution_graph` record goes.
///
/// It has to go at the JSON-value layer, before `ApprovalState` deserializes:
/// left in place the record would not read as an unknown gate to be
/// normalized away, it would fail the whole file to parse, and a user's
/// approvals would be unreadable rather than merely one gate shorter.
#[test]
fn a_v1_approvals_file_drops_only_the_execution_graph_gate() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    let path = layout.planning_dir(&name).join("approvals.json");
    fs::ensure_dir(path.parent().unwrap()).unwrap();
    fs::write_text(
        &path,
        &serde_json::to_string(&serde_json::json!({
            "version": 1,
            "gates": [
                {"gate": "requirements", "state": "approved", "artifact_fingerprint": "fp-req"},
                {"gate": "analysis", "state": "approved", "artifact_fingerprint": "fp-ana"},
                {"gate": "plan", "state": "needs_revision", "artifact_fingerprint": "fp-plan"},
                {"gate": "execution_graph", "state": "approved", "artifact_fingerprint": "fp-graph"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    let migrated = ApprovalState::read(&layout, &name)
        .unwrap()
        .expect("a v1 approvals file must migrate, not fail");

    assert_eq!(migrated.gates.len(), 3);
    assert_eq!(
        migrated.state(Gate::Requirements),
        Some(GateState::Approved)
    );
    assert_eq!(migrated.state(Gate::Analysis), Some(GateState::Approved));
    assert_eq!(migrated.state(Gate::Plan), Some(GateState::NeedsRevision));
    assert_eq!(
        migrated
            .record(Gate::Plan)
            .and_then(|record| record.artifact_fingerprint.as_deref()),
        Some("fp-plan"),
        "fingerprints survive the migration untouched"
    );

    let text = fs::read_text(&path).unwrap().unwrap();
    assert!(
        text.contains("\"version\": 2"),
        "the migrated form must persist as v2: {text}"
    );
    assert!(
        !text.contains("execution_graph"),
        "the retired gate must be gone from disk: {text}"
    );
}

/// A v1 file whose fourth gate was never written is not a special case — the
/// retain simply removes nothing.
#[test]
fn a_v1_approvals_file_without_the_retired_gate_migrates_unchanged() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    let path = layout.planning_dir(&name).join("approvals.json");
    fs::ensure_dir(path.parent().unwrap()).unwrap();
    fs::write_text(
        &path,
        r#"{"version":1,"gates":[{"gate":"requirements","state":"approved","artifact_fingerprint":null}]}"#,
    )
    .unwrap();

    let migrated = ApprovalState::read(&layout, &name).unwrap().unwrap();

    assert_eq!(migrated.gates.len(), 1);
    assert_eq!(
        migrated.state(Gate::Requirements),
        Some(GateState::Approved)
    );
}

#[test]
fn an_approvals_file_newer_than_the_binary_is_refused() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    let path = layout.planning_dir(&name).join("approvals.json");
    fs::ensure_dir(path.parent().unwrap()).unwrap();
    fs::write_text(&path, r#"{"version":99,"gates":[]}"#).unwrap();

    assert_eq!(
        ApprovalState::read(&layout, &name).unwrap_err().code,
        "store.version_too_new"
    );
}

// -- run receipts ----------------------------------------------------------

fn run_id(tail: &str) -> RunId {
    RunId::new(format!("6f1d9e64-0d1a-4f2b-9a5c-2b7e1d4c8a{tail}")).unwrap()
}

fn session() -> SessionId {
    SessionId::new("11111111-2222-3333-4444-555555555501").unwrap()
}

fn report() -> CoordinatorReport {
    CoordinatorReport {
        summary: "landed the receipt store".to_owned(),
        tasks: vec![TaskResult {
            title: "persist run.json".to_owned(),
            status: TaskStatus::Completed,
            result: "round-trips".to_owned(),
        }],
        verification: vec![VerificationCheck {
            command: "cargo test".to_owned(),
            status: CheckStatus::Passed,
            summary: "green".to_owned(),
        }],
        agents: Vec::new(),
        deviations: Vec::new(),
        blockers: Vec::new(),
        follow_ups: Vec::new(),
    }
}

/// An active receipt for `checkout`, started at `at`.
fn receipt(id: RunId, at: &str) -> RunReceipt {
    RunReceipt::start(
        id,
        FeatureName::new("checkout").unwrap(),
        "plans/checkout/plan.md",
        "plan-fp-1",
        RunBaseline::empty(),
        session(),
        Provider::ClaudeCode,
        at,
    )
}

/// A terminal receipt — what the archive is for.
fn finished(id: RunId, at: &str) -> RunReceipt {
    let mut receipt = receipt(id, at);
    receipt
        .terminate(
            RunOutcome::Succeeded,
            report(),
            RunDiff::default(),
            session(),
            Provider::ClaudeCode,
            at,
        )
        .unwrap();
    receipt
}

#[test]
fn absent_receipt_reads_as_ok_none() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();

    assert_eq!(RunReceipt::read(&layout, &name).unwrap(), None);
    assert_eq!(
        RunReceipt::read_archived(&layout, &name, &run_id("01")).unwrap(),
        None
    );
    assert_eq!(
        RunReceipt::find(&layout, &name, &run_id("01")).unwrap(),
        None
    );
    assert!(run::history(&layout, &name).unwrap().is_empty());
}

#[test]
fn a_receipt_write_then_read_round_trips() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    let receipt = receipt(run_id("01"), "T0");

    receipt.write(&layout).unwrap();

    assert_eq!(
        RunReceipt::read(&layout, &name).unwrap().as_ref(),
        Some(&receipt)
    );
}

#[test]
fn the_receipt_lands_in_the_execution_directory_as_run_json() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();

    receipt(run_id("01"), "T0").write(&layout).unwrap();

    assert_eq!(
        run::current_path(&layout, &name),
        layout.execution_dir(&name).join("run.json")
    );
    assert!(fs::is_file(&run::current_path(&layout, &name)).unwrap());
}

#[test]
fn the_written_receipt_is_canonical_and_version_stamped() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();

    receipt(run_id("01"), "T0").write(&layout).unwrap();

    let text = fs::read_text(&run::current_path(&layout, &name))
        .unwrap()
        .unwrap();
    assert!(
        text.contains("\"version\": 1"),
        "the store must stamp the version: {text}"
    );
    assert!(text.contains("\"status\": \"active\""));
    assert!(text.ends_with('\n'), "canonical JSON ends with a newline");
}

#[test]
fn a_receipt_newer_than_the_binary_is_refused() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    let path = run::current_path(&layout, &name);
    fs::ensure_dir(path.parent().unwrap()).unwrap();
    fs::write_text(&path, r#"{"version":99,"id":"x"}"#).unwrap();

    assert_eq!(
        RunReceipt::read(&layout, &name).unwrap_err().code,
        "store.version_too_new"
    );
}

/// `run.json`'s chain starts at v1: there is no unversioned predecessor, so a
/// file with no `version` field is refused on schema grounds rather than
/// adopted as v0 and silently migrated into something it never was.
#[test]
fn an_unversioned_receipt_file_is_not_adopted_as_v0() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    let path = run::current_path(&layout, &name);
    fs::ensure_dir(path.parent().unwrap()).unwrap();
    fs::write_text(&path, r#"{"id":"6f1d9e64-0d1a-4f2b-9a5c-2b7e1d4c8a01"}"#).unwrap();

    assert!(RunReceipt::read(&layout, &name).is_err());
}

#[test]
fn archiving_a_terminal_receipt_makes_it_readable_by_id() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    let receipt = finished(run_id("01"), "T0");

    let path = run::archive(&layout, &receipt).unwrap();

    assert_eq!(path, layout.archived_run(&name, &run_id("01")));
    assert_eq!(
        RunReceipt::read_archived(&layout, &name, &run_id("01"))
            .unwrap()
            .as_ref(),
        Some(&receipt)
    );
}

/// The archive is for runs that are over. A live run copied into it would be a
/// second, frozen truth about a run still changing.
#[test]
fn archiving_a_live_run_is_refused() {
    let (_guard, layout) = layout_with_hall();
    let receipt = receipt(run_id("01"), "T0");

    let failure = run::archive(&layout, &receipt).unwrap_err();

    assert_eq!(failure.code, "execute.archive_live_run");
    assert_eq!(failure.fix_actions.len(), 1);
}

/// What makes archive-then-remove restartable: a crash between the two steps
/// leaves the archive written, and the next call writes the identical bytes
/// again rather than failing.
#[test]
fn archiving_the_same_receipt_twice_is_a_no_op() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    let receipt = finished(run_id("01"), "T0");

    let first = run::archive(&layout, &receipt).unwrap();
    let second = run::archive(&layout, &receipt).unwrap();

    assert_eq!(first, second);
    assert_eq!(
        RunReceipt::read_archived(&layout, &name, &run_id("01"))
            .unwrap()
            .as_ref(),
        Some(&receipt)
    );
}

/// Idempotent only for *identical* content. An archived receipt is evidence,
/// and a different run under an id already spoken for is refused rather than
/// overwritten.
#[test]
fn archiving_different_content_under_an_existing_id_is_refused() {
    let (_guard, layout) = layout_with_hall();
    let first = finished(run_id("01"), "T0");
    let mut second = finished(run_id("01"), "T0");
    second.plan_fingerprint = "plan-fp-2".to_owned();

    run::archive(&layout, &first).unwrap();
    let failure = run::archive(&layout, &second).unwrap_err();

    assert_eq!(failure.code, "execute.archive_conflict");
}

#[test]
fn archiving_the_current_run_clears_run_json() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    let receipt = finished(run_id("01"), "T0");
    receipt.write(&layout).unwrap();

    let archived = run::archive_current(&layout, &name).unwrap();

    assert_eq!(archived, Some(run_id("01")));
    assert_eq!(RunReceipt::read(&layout, &name).unwrap(), None);
    assert!(!fs::exists(&run::current_path(&layout, &name)).unwrap());
    assert_eq!(
        RunReceipt::read_archived(&layout, &name, &run_id("01"))
            .unwrap()
            .as_ref(),
        Some(&receipt)
    );
}

/// The state a crash between archive and remove leaves: the receipt is in both
/// places. Running it again finds an identical archive entry, no-ops, and
/// removes `run.json` — so the next run has its slot.
#[test]
fn archiving_the_current_run_recovers_from_a_crash_after_the_archive_write() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    let receipt = finished(run_id("01"), "T0");
    receipt.write(&layout).unwrap();
    run::archive(&layout, &receipt).unwrap();

    assert_eq!(
        run::archive_current(&layout, &name).unwrap(),
        Some(run_id("01"))
    );
    assert_eq!(RunReceipt::read(&layout, &name).unwrap(), None);
}

#[test]
fn archiving_the_current_run_when_there_is_none_answers_none() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();

    assert_eq!(run::archive_current(&layout, &name).unwrap(), None);
}

/// A terminal receipt is archived before the next current run replaces it, so
/// both survive and one file holds one run.
#[test]
fn a_terminal_receipt_is_archived_before_the_next_current_run() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    finished(run_id("01"), "T0").write(&layout).unwrap();

    run::archive_current(&layout, &name).unwrap();
    let next = receipt(run_id("02"), "T1");
    next.write(&layout).unwrap();

    assert_eq!(
        RunReceipt::read(&layout, &name).unwrap().as_ref(),
        Some(&next)
    );
    assert!(
        RunReceipt::read_archived(&layout, &name, &run_id("01"))
            .unwrap()
            .is_some()
    );
}

/// Current first: an id that is both current and archived would mean an
/// archive was written before the run ended, and the live value is the one to
/// report.
#[test]
fn find_looks_at_the_current_run_before_the_archive() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    let current = receipt(run_id("01"), "T0");
    current.write(&layout).unwrap();
    let archived = finished(run_id("02"), "T1");
    run::archive(&layout, &archived).unwrap();

    assert_eq!(
        RunReceipt::find(&layout, &name, &run_id("01"))
            .unwrap()
            .as_ref(),
        Some(&current)
    );
    assert_eq!(
        RunReceipt::find(&layout, &name, &run_id("02"))
            .unwrap()
            .as_ref(),
        Some(&archived)
    );
    assert_eq!(
        RunReceipt::find(&layout, &name, &run_id("03")).unwrap(),
        None
    );
}

/// Newest first, with the id breaking ties, so two listings of the same
/// directory read identically no matter what order the filesystem hands the
/// entries back.
#[test]
fn history_is_newest_first_and_includes_the_current_run() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    run::archive(&layout, &finished(run_id("01"), "2026-08-01T00:00:00Z")).unwrap();
    run::archive(&layout, &finished(run_id("03"), "2026-08-03T00:00:00Z")).unwrap();
    run::archive(&layout, &finished(run_id("02"), "2026-08-02T00:00:00Z")).unwrap();
    receipt(run_id("04"), "2026-08-04T00:00:00Z")
        .write(&layout)
        .unwrap();

    let ids: Vec<_> = run::history(&layout, &name)
        .unwrap()
        .into_iter()
        .map(|receipt| receipt.id)
        .collect();

    assert_eq!(
        ids,
        vec![run_id("04"), run_id("03"), run_id("02"), run_id("01")]
    );
}

/// Two runs started in the same instant still order deterministically — the id
/// breaks the tie, and nothing is left to the directory walk.
#[test]
fn history_breaks_a_timestamp_tie_by_run_id() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    run::archive(&layout, &finished(run_id("01"), "T0")).unwrap();
    run::archive(&layout, &finished(run_id("02"), "T0")).unwrap();

    let ids: Vec<_> = run::history(&layout, &name)
        .unwrap()
        .into_iter()
        .map(|receipt| receipt.id)
        .collect();

    assert_eq!(ids, vec![run_id("02"), run_id("01")]);
}

#[test]
fn history_steps_over_files_that_are_not_receipts() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    run::archive(&layout, &finished(run_id("01"), "T0")).unwrap();
    let dir = layout.run_archive_runs_dir(&name);
    fs::write_text(&dir.join("notes.txt"), "scratch").unwrap();
    fs::write_text(&dir.join("not-a-uuid.json"), "{}").unwrap();

    assert_eq!(run::history(&layout, &name).unwrap().len(), 1);
}

// -- legacy board import ---------------------------------------------------

/// A v3 board — the last shape ivar ever wrote — as raw JSON, so the import
/// path is exercised end to end rather than through `ExecutionBoard`.
fn v3_board(status: &str) -> serde_json::Value {
    serde_json::json!({
        "version": 3,
        "status": status,
        "blocked_by": null,
        "next_event_seq": 3,
        "sessions": {"sess-a": "receipt-core"},
        "graph": {
            "plan_fingerprint": "board-fp",
            "workstreams": [
                {
                    "id": "receipt-core",
                    "title": "Run Receipt domain",
                    "operations": ["OP-RUN-DOMAIN", "OP-RUN-STORE"],
                    "depends_on": [],
                    "write_contract": ["src/domain/feature/**"],
                    "status": "active",
                    "provider": "claude-code",
                    "model": null,
                    "agent": null
                },
                {
                    "id": "receipt-actions",
                    "title": "Run lifecycle actions",
                    "operations": ["OP-EXECUTE-START"],
                    "depends_on": ["receipt-core"],
                    "write_contract": ["src/action/execute/**"],
                    "status": "waiting",
                    "provider": null,
                    "model": null,
                    "agent": null
                }
            ]
        },
        "journal": [
            {
                "seq": 1,
                "event_id": "prepared.1",
                "timestamp": "1787416313",
                "workstream": "board",
                "kind": "prepared",
                "message": "Execution board prepared",
                "revision": null
            },
            {
                "seq": 2,
                "event_id": "started.1",
                "timestamp": "1787416400",
                "workstream": "receipt-core",
                "kind": "started",
                "message": "Launched session",
                "revision": null
            }
        ]
    })
}

fn write_board(layout: &Layout, name: &FeatureName, board: &serde_json::Value) {
    fs::ensure_dir(&layout.execution_dir(name)).unwrap();
    fs::write_text(
        &board_path(layout, name),
        &serde_json::to_string_pretty(board).unwrap(),
    )
    .unwrap();
}

fn import(layout: &Layout, name: &FeatureName, id: &str) -> Option<run::Import> {
    run::import(
        layout,
        name,
        "plans/checkout/plan.md",
        run_id(id),
        "2026-08-22T00:00:00Z",
    )
    .unwrap()
}

#[test]
fn importing_with_no_board_at_all_answers_none() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();

    assert_eq!(import(&layout, &name, "01"), None);
}

/// A running board is not a run that can be continued: the workstreams, their
/// dependency waves and their per-workstream sessions have no faithful mapping
/// onto a provider-native coordinator.
#[test]
fn a_non_terminal_board_imports_as_an_interrupted_receipt() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    write_board(&layout, &name, &v3_board("running"));

    let imported = import(&layout, &name, "01").unwrap();

    assert_eq!(imported.receipt.status, RunStatus::Interrupted);
    assert_eq!(imported.receipt.outcome, None);
    assert_eq!(imported.receipt.provenance, RunProvenance::LegacyImport);
    assert!(!imported.receipt.holds_lock());
    assert!(!imported.resumed);
}

#[test]
fn every_old_board_status_maps_to_a_receipt_status() {
    for (board_status, expected, outcome) in [
        (
            "completed",
            RunStatus::Succeeded,
            Some(RunOutcome::Succeeded),
        ),
        ("failed", RunStatus::Failed, Some(RunOutcome::Failed)),
        ("pending", RunStatus::Interrupted, None),
        ("awaiting_approval", RunStatus::Interrupted, None),
        ("approved", RunStatus::Interrupted, None),
        ("running", RunStatus::Interrupted, None),
        ("blocked", RunStatus::Interrupted, None),
        ("paused", RunStatus::Interrupted, None),
        ("a_status_no_ivar_ever_wrote", RunStatus::Interrupted, None),
    ] {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();
        write_board(&layout, &name, &v3_board(board_status));

        let imported = import(&layout, &name, "01").unwrap();

        assert_eq!(
            imported.receipt.status, expected,
            "board status `{board_status}`"
        );
        assert_eq!(
            imported.receipt.outcome, outcome,
            "board status `{board_status}`"
        );
    }
}

/// The whole point of the legacy payload: `status` can say what was there
/// without anyone opening the archived board by hand.
#[test]
fn graph_session_and_journal_evidence_survives_the_import() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    write_board(&layout, &name, &v3_board("running"));

    let imported = import(&layout, &name, "01").unwrap();
    let legacy = imported.receipt.legacy.as_ref().unwrap();

    assert_eq!(legacy.board_status, "running");
    assert_eq!(legacy.plan_fingerprint.as_deref(), Some("board-fp"));
    assert_eq!(legacy.workstreams.len(), 2);
    assert_eq!(legacy.workstreams[0].id, "receipt-core");
    assert_eq!(legacy.workstreams[0].status, "active");
    assert_eq!(
        legacy.workstreams[0].operations,
        vec!["OP-RUN-DOMAIN".to_owned(), "OP-RUN-STORE".to_owned()]
    );
    assert_eq!(
        legacy.workstreams[1].depends_on,
        vec!["receipt-core".to_owned()]
    );
    assert_eq!(
        legacy.sessions.get("sess-a").map(String::as_str),
        Some("receipt-core")
    );
    assert_eq!(legacy.journal.len(), 2);
    assert_eq!(legacy.journal[1].kind, "started");
    assert_eq!(legacy.journal[1].seq, 2);
    assert_eq!(legacy.archived_board, imported.archived_board);
}

/// Every schema version the board was ever written at still imports — N-COMPAT
/// is a permanent contract, not a deprecation window, so the chain runs even
/// though nothing will write a board again.
#[test]
fn every_board_schema_version_imports() {
    // v0: no `version` field at all.
    let v0 = serde_json::json!({
        "status": "running",
        "graph": {
            "plan_fingerprint": "v0-fp",
            "workstreams": [{
                "id": "ws1", "title": "WS one", "operations": ["op1"],
                "depends_on": [], "write_contract": ["src/"], "status": "active"
            }]
        },
        "journal": [{
            "kind": "prepared", "timestamp": "1700000000",
            "workstream": "board", "message": "board prepared"
        }]
    });
    let mut v1 = v0.clone();
    v1["version"] = serde_json::json!(1);
    let mut v2 = v3_board("running");
    v2["version"] = serde_json::json!(2);
    for entry in v2["journal"].as_array_mut().unwrap() {
        entry.as_object_mut().unwrap().remove("revision");
    }

    for (label, board, fingerprint) in [
        ("v0", v0, "v0-fp"),
        ("v1", v1, "v0-fp"),
        ("v2", v2, "board-fp"),
        ("v3", v3_board("running"), "board-fp"),
    ] {
        let (_guard, layout) = layout_with_hall();
        let name = FeatureName::new("checkout").unwrap();
        write_board(&layout, &name, &board);

        let imported =
            import(&layout, &name, "01").unwrap_or_else(|| panic!("{label} board must import"));
        let legacy = imported.receipt.legacy.as_ref().unwrap();

        assert_eq!(
            legacy.plan_fingerprint.as_deref(),
            Some(fingerprint),
            "{label}"
        );
        assert!(!legacy.workstreams.is_empty(), "{label}");
        assert!(!legacy.journal.is_empty(), "{label}");
    }
}

#[test]
fn a_board_newer_than_the_binary_is_refused_rather_than_guessed_at() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    let mut board = v3_board("running");
    board["version"] = serde_json::json!(99);
    write_board(&layout, &name, &board);

    let failure =
        run::import(&layout, &name, "plans/checkout/plan.md", run_id("01"), "T0").unwrap_err();

    assert_eq!(failure.code, "store.version_too_new");
}

/// A completed import leaves the receipt archived, the board archived, and no
/// `board.json` — which is also the state that makes a second call a no-op.
#[test]
fn a_completed_import_archives_everything_and_removes_the_board() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    write_board(&layout, &name, &v3_board("completed"));

    let imported = import(&layout, &name, "01").unwrap();

    assert!(!fs::exists(&board_path(&layout, &name)).unwrap());
    assert!(!fs::exists(&run::current_path(&layout, &name)).unwrap());
    assert!(fs::is_file(&imported.archived_board).unwrap());
    assert_eq!(
        RunReceipt::read_archived(&layout, &name, &run_id("01"))
            .unwrap()
            .as_ref(),
        Some(&imported.receipt)
    );
    assert_eq!(import(&layout, &name, "02"), None, "the board is gone");
}

/// Crash point 1 — board only. Nothing was written yet, so a restart performs
/// the whole import.
#[test]
fn an_import_that_crashed_before_writing_anything_runs_from_scratch() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    write_board(&layout, &name, &v3_board("running"));

    let imported = import(&layout, &name, "01").unwrap();

    assert!(!imported.resumed);
    assert_eq!(imported.receipt.id, run_id("01"));
}

/// Crash point 2 — receipt + board. `run.json` holds the imported receipt and
/// `board.json` is still there. The restart must finish that import, reusing
/// its id, rather than minting a second run for the same board.
#[test]
fn an_import_that_crashed_after_writing_run_json_resumes_with_the_same_id() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    let board = v3_board("running");
    write_board(&layout, &name, &board);
    let first = import(&layout, &name, "01").unwrap();

    // Rebuild the interrupted shape: receipt current again, board back.
    fs::remove_file(&layout.archived_run(&name, &run_id("01"))).unwrap();
    first.receipt.write(&layout).unwrap();
    write_board(&layout, &name, &board);

    let resumed = import(&layout, &name, "99").unwrap();

    assert!(resumed.resumed);
    assert_eq!(
        resumed.receipt.id,
        run_id("01"),
        "the id must not be reminted"
    );
    assert_eq!(resumed.receipt, first.receipt);
    assert!(!fs::exists(&board_path(&layout, &name)).unwrap());
    assert_eq!(run::history(&layout, &name).unwrap().len(), 1);
}

/// Crash point 3 — receipt + archive + board. The receipt is already archived
/// and `board.json` survived the crash; the restart removes it and changes
/// nothing else.
#[test]
fn an_import_that_crashed_before_removing_the_board_just_removes_it() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    let board = v3_board("running");
    write_board(&layout, &name, &board);
    let first = import(&layout, &name, "01").unwrap();
    write_board(&layout, &name, &board);

    let resumed = import(&layout, &name, "99").unwrap();

    assert!(resumed.resumed);
    assert_eq!(resumed.receipt, first.receipt);
    assert!(!fs::exists(&board_path(&layout, &name)).unwrap());
    assert_eq!(run::history(&layout, &name).unwrap().len(), 1);
}

/// Importing twice from the identical board never doubles the history — which
/// is the property every crash point above reduces to.
#[test]
fn repeated_imports_of_the_same_board_never_double_the_history() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    let board = v3_board("completed");

    for _ in 0..3 {
        write_board(&layout, &name, &board);
        import(&layout, &name, "01").unwrap();
    }

    assert_eq!(run::history(&layout, &name).unwrap().len(), 1);
}

/// The source hash is what tells a continuation apart from a conflict: the
/// board on disk changed after an import began, and merging two boards into
/// one history is not something to do silently.
#[test]
fn a_board_that_changed_under_a_half_finished_import_is_refused() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    write_board(&layout, &name, &v3_board("running"));
    let first = import(&layout, &name, "01").unwrap();

    // Put the import back into its crash-point-2 shape, then swap the board.
    fs::remove_file(&layout.archived_run(&name, &run_id("01"))).unwrap();
    first.receipt.write(&layout).unwrap();
    write_board(&layout, &name, &v3_board("failed"));

    let failure =
        run::import(&layout, &name, "plans/checkout/plan.md", run_id("02"), "T1").unwrap_err();

    assert_eq!(failure.code, "execute.legacy_source_conflict");
}

/// A coordinator is holding the feature right now. Writing an imported receipt
/// into `run.json` would take the lock out from under it.
#[test]
fn importing_while_a_native_run_is_in_flight_is_refused() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    receipt(run_id("01"), "T0").write(&layout).unwrap();
    write_board(&layout, &name, &v3_board("running"));

    let failure =
        run::import(&layout, &name, "plans/checkout/plan.md", run_id("02"), "T1").unwrap_err();

    assert_eq!(failure.code, "execute.legacy_import_blocked");
    assert!(fs::exists(&board_path(&layout, &name)).unwrap());
}

/// Content-addressed by construction, so different content computes a
/// different name and cannot collide. The refusal is the assertion that this
/// still holds, not a merge strategy.
#[test]
fn a_board_archive_is_never_overwritten_with_different_bytes() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    write_board(&layout, &name, &v3_board("running"));
    let imported = import(&layout, &name, "01").unwrap();

    // Forge the collision the content addressing is supposed to make
    // impossible: same path, different bytes.
    fs::write_text(&imported.archived_board, "{\"tampered\":true}\n").unwrap();
    write_board(&layout, &name, &v3_board("running"));

    let failure =
        run::import(&layout, &name, "plans/checkout/plan.md", run_id("02"), "T1").unwrap_err();

    assert_eq!(failure.code, "execute.board_archive_conflict");
}

/// The board is preserved whole and unmodified — the receipt's evidence is a
/// summary, and the archive is the thing to go back to.
#[test]
fn the_archived_board_holds_the_normalized_board_itself() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    write_board(&layout, &name, &v3_board("running"));

    let imported = import(&layout, &name, "01").unwrap();

    let text = fs::read_text(&imported.archived_board).unwrap().unwrap();
    let archived: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(archived["version"], serde_json::json!(3));
    assert_eq!(archived["status"], serde_json::json!("running"));
    assert_eq!(
        archived["graph"]["workstreams"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        imported.archived_board,
        layout.archived_board(
            &name,
            imported
                .receipt
                .legacy
                .as_ref()
                .unwrap()
                .source_hash
                .as_str()
        )
    );
}

/// A v0/v1 board is normalized *before* it is hashed and archived, so the
/// archive holds one shape regardless of which version it arrived as.
#[test]
fn the_archived_board_is_normalized_to_the_last_board_schema() {
    let (_guard, layout) = layout_with_hall();
    let name = FeatureName::new("checkout").unwrap();
    write_board(
        &layout,
        &name,
        &serde_json::json!({
            "status": "running",
            "graph": {"plan_fingerprint": "v0-fp", "workstreams": []},
            "journal": [{
                "kind": "prepared", "timestamp": "1700000000",
                "workstream": "board", "message": "board prepared"
            }]
        }),
    );

    let imported = import(&layout, &name, "01").unwrap();

    let text = fs::read_text(&imported.archived_board).unwrap().unwrap();
    let archived: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(archived["version"], serde_json::json!(3));
    assert_eq!(archived["journal"][0]["seq"], serde_json::json!(1));
    assert_eq!(archived["journal"][0]["revision"], serde_json::Value::Null);
    assert!(archived["sessions"].is_object());
}
