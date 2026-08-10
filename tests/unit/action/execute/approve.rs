#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::execute::prepare::{self as prepare_action, PrepareInput};
use crate::action::feature::create::{
    self as feature_create, CreateInput as FeatureCreateInput,
};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
use crate::domain::feature::ExecutionStatus;
use crate::error::Status;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::test_support::hall_root;

const GRAPH_JSON: &str = r#"{
    "workstreams": [
        {
            "id": "ws-gates",
            "title": "Approval gates",
            "operations": ["add-gate-types", "wire-approve"],
            "depends_on": [],
            "write_contract": ["src/domain/feature.rs"]
        },
        {
            "id": "ws-board",
            "title": "Execution board",
            "operations": ["add-board-types", "store-board"],
            "depends_on": ["ws-gates"],
            "write_contract": ["src/action/execute"]
        }
    ]
}"#;

/// A hall with a feature, a plan, and a prepared board.
fn seeded_board() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();
    feature_create::create(
        &ctx,
        FeatureCreateInput {
            name: "checkout".to_owned(),
            branch: None,
        },
    )
    .unwrap();
    plan_create::create(
        &ctx,
        PlanCreateInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    let graph = root.join("graph.json");
    fs::write_text(&graph, GRAPH_JSON).unwrap();
    prepare_action::prepare(
        &ctx,
        PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: graph.to_string(),
        },
    )
    .unwrap();
    (guard, root)
}

/// The board read back off disk — the real file, not the in-memory value
/// an action returned.
fn persisted(root: &Utf8PathBuf) -> ExecutionBoard {
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    ExecutionBoard::read(&layout, &feature).unwrap().unwrap()
}

/// The persisted approval state, read back off disk.
fn persisted_approvals(root: &Utf8PathBuf) -> ApprovalState {
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    ApprovalState::read(&layout, &feature).unwrap().unwrap()
}

#[test]
fn approve_transitions_awaiting_approval_to_approved_and_closes_the_gate() {
    let (_guard, root) = seeded_board();
    let ctx = Ctx::new(root.clone());

    let report = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.board.status, ExecutionStatus::Approved);

    // The journal contains the graph.approved entry.
    let on_disk = persisted(&root);
    let last_entry = on_disk.journal.last().unwrap();
    assert_eq!(last_entry.kind, "graph.approved");
    assert!(!last_entry.event_id.is_empty());

    // The Execution Graph gate is closed in approvals.
    let approvals = persisted_approvals(&root);
    assert_eq!(
        approvals.state(Gate::ExecutionGraph),
        Some(GateState::Approved)
    );
}

#[test]
fn approve_refuses_a_board_not_in_awaiting_approval_naming_the_state() {
    let (_guard, root) = seeded_board();
    let ctx = Ctx::new(root.clone());

    // Manually change the board to Pending via store.
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
    board.set_status(ExecutionStatus::Pending);
    board.write(&layout, &feature).unwrap();

    let failure = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "execute.board_not_awaiting_approval");
    assert!(
        failure.what.contains("pending"),
        "error must name the actual state: {}",
        failure.what
    );
}

#[test]
fn approve_twice_does_not_duplicate_the_journal_event() {
    let (_guard, root) = seeded_board();
    let ctx = Ctx::new(root.clone());

    // First approve.
    approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let journal_len_after_first = persisted(&root).journal.len();

    // Second approve — should be a no-op.
    approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let journal_len_after_second = persisted(&root).journal.len();

    assert_eq!(
        journal_len_after_first, journal_len_after_second,
        "second approve must not add a journal entry"
    );
}

#[test]
fn after_execute_approve_the_execution_graph_gate_is_approved() {
    let (_guard, root) = seeded_board();
    let ctx = Ctx::new(root.clone());

    approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let approvals = persisted_approvals(&root);

    // The three SPDD gates upstream are untouched — `plan approve` owns
    // them, and `execute approve` writes only the execution-graph gate.
    assert_eq!(
        approvals.state(Gate::Requirements),
        Some(GateState::Pending)
    );
    assert_eq!(approvals.state(Gate::Analysis), Some(GateState::Pending));
    assert_eq!(approvals.state(Gate::Plan), Some(GateState::Pending));
    assert_eq!(
        approvals.state(Gate::ExecutionGraph),
        Some(GateState::Approved)
    );
}

#[test]
fn the_human_surface_lists_journal_entries() {
    let outcome = ApproveOutcome {
        root: Utf8PathBuf::from("/hall"),
        feature: FeatureName::new("checkout").unwrap(),
        board_path: Utf8PathBuf::from("/hall/board.json"),
        board: ExecutionBoard::new(crate::domain::feature::ExecutionGraph {
            plan_fingerprint: "abc".to_owned(),
            workstreams: vec![],
        }),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Approved execution board"));
    assert!(text.contains("checkout"));
}
