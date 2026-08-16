//! Unit tests for `crate::domain::feature::approval` — the four SPDD
//! approval gates and their state.
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

// -- approval gates ---------------------------------------------------------

#[test]
fn the_four_gates_form_a_chain_in_lifecycle_order() {
    assert_eq!(
        Gate::ALL,
        [
            Gate::Requirements,
            Gate::Analysis,
            Gate::Plan,
            Gate::ExecutionGraph
        ]
    );
    assert_eq!(Gate::Requirements.upstream(), None);
    assert_eq!(Gate::Analysis.upstream(), Some(Gate::Requirements));
    assert_eq!(Gate::Plan.upstream(), Some(Gate::Analysis));
    assert_eq!(Gate::ExecutionGraph.upstream(), Some(Gate::Plan));
}

#[test]
fn and_downstream_lists_the_gate_and_everything_after_it() {
    assert_eq!(
        Gate::Requirements.and_downstream(),
        &[
            Gate::Requirements,
            Gate::Analysis,
            Gate::Plan,
            Gate::ExecutionGraph
        ]
    );
    assert_eq!(
        Gate::Analysis.and_downstream(),
        &[Gate::Analysis, Gate::Plan, Gate::ExecutionGraph]
    );
    assert_eq!(
        Gate::Plan.and_downstream(),
        &[Gate::Plan, Gate::ExecutionGraph]
    );
    assert_eq!(
        Gate::ExecutionGraph.and_downstream(),
        &[Gate::ExecutionGraph]
    );
}

#[test]
fn gate_parse_accepts_every_cli_name_and_rejects_unknowns() {
    assert_eq!(Gate::parse("requirements"), Ok(Gate::Requirements));
    assert_eq!(Gate::parse("analysis"), Ok(Gate::Analysis));
    assert_eq!(Gate::parse("plan"), Ok(Gate::Plan));
    assert_eq!(Gate::parse("execution-graph"), Ok(Gate::ExecutionGraph));
    assert_eq!(Gate::parse("execution_graph"), Ok(Gate::ExecutionGraph));
    assert!(matches!(Gate::parse("bogus"), Err(UnknownGate(_))));
}

#[test]
fn display_names_are_the_cli_surface() {
    assert_eq!(Gate::Requirements.to_string(), "requirements");
    assert_eq!(Gate::Analysis.to_string(), "analysis");
    assert_eq!(Gate::Plan.to_string(), "plan");
    assert_eq!(Gate::ExecutionGraph.to_string(), "execution-graph");
    assert_eq!(GateState::Pending.to_string(), "pending");
    assert_eq!(GateState::Approved.to_string(), "approved");
    assert_eq!(GateState::NeedsRevision.to_string(), "needs-revision");
}

#[test]
fn serde_names_are_snake_case() {
    assert_eq!(
        serde_json::to_value(Gate::ExecutionGraph).unwrap(),
        serde_json::json!("execution_graph")
    );
    assert_eq!(
        serde_json::to_value(GateState::NeedsRevision).unwrap(),
        serde_json::json!("needs_revision")
    );
}

#[test]
fn fresh_approval_state_has_all_four_gates_pending() {
    let approvals = ApprovalState::fresh();

    assert_eq!(approvals.gates.len(), 4);
    for gate in Gate::ALL {
        assert_eq!(approvals.state(gate), Some(GateState::Pending));
    }
}

#[test]
fn set_updates_an_existing_record_and_normalize_fills_gaps() {
    let mut approvals = ApprovalState::fresh();
    approvals.set(
        Gate::Requirements,
        GateState::Approved,
        Some("fp".to_owned()),
    );

    assert_eq!(
        approvals.state(Gate::Requirements),
        Some(GateState::Approved)
    );
    assert_eq!(
        approvals
            .record(Gate::Requirements)
            .unwrap()
            .artifact_fingerprint
            .as_deref(),
        Some("fp")
    );

    // A hand-edited file may carry fewer gates; normalize completes them.
    let mut partial = ApprovalState { gates: Vec::new() };
    partial.normalize();
    assert_eq!(partial.gates.len(), 4);
    assert_eq!(
        partial.state(Gate::ExecutionGraph),
        Some(GateState::Pending)
    );
}

#[test]
fn upstream_approved_tracks_the_chain() {
    let mut approvals = ApprovalState::fresh();

    assert!(approvals.upstream_approved(Gate::Requirements));
    assert!(!approvals.upstream_approved(Gate::Analysis));

    approvals.set(Gate::Requirements, GateState::Approved, None);

    assert!(approvals.upstream_approved(Gate::Analysis));
    assert!(!approvals.upstream_approved(Gate::Plan));
}

#[test]
fn invalidate_from_marks_the_gate_and_downstream_and_clears_fingerprints() {
    let mut approvals = ApprovalState::fresh();
    for gate in Gate::ALL {
        approvals.set(gate, GateState::Approved, Some(format!("fp-{gate}")));
    }

    approvals.invalidate_from(Gate::Analysis);

    assert_eq!(
        approvals.state(Gate::Requirements),
        Some(GateState::Approved)
    );
    for gate in [Gate::Analysis, Gate::Plan, Gate::ExecutionGraph] {
        assert_eq!(approvals.state(gate), Some(GateState::NeedsRevision));
        assert_eq!(approvals.record(gate).unwrap().artifact_fingerprint, None);
    }
}

#[test]
fn approval_state_round_trips_through_serde() {
    let mut approvals = ApprovalState::fresh();
    approvals.set(
        Gate::Requirements,
        GateState::Approved,
        Some("abc".to_owned()),
    );

    let rendered = serde_json::to_string(&approvals).unwrap();
    let parsed: ApprovalState = serde_json::from_str(&rendered).unwrap();

    assert_eq!(parsed, approvals);
}
