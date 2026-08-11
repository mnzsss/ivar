#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::execute::approve as execute_approve;
use crate::action::execute::prepare as execute_prepare;
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::approve::{self as plan_approve, ApproveInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
use crate::error::Status;
use crate::test_support::hall_root;

const GRAPH_JSON: &str = r#"{
    "workstreams": [
        {
            "id": "ws-gates",
            "title": "Approval gates",
            "operations": ["add-gate-types", "wire-approve"],
            "depends_on": [],
            "write_contract": ["src/domain/feature.rs"]
        }
    ]
}"#;

fn seeded_hall() -> (tempfile::TempDir, Utf8PathBuf) {
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
    (guard, root)
}

fn status_input(path: &str) -> StatusInput {
    StatusInput {
        plan_path: path.to_owned(),
    }
}

fn approve_gate(ctx: &Ctx, gate: &str) {
    plan_approve::approve(
        ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: gate.to_owned(),
        },
    )
    .unwrap();
}

/// Put a freshly prepared board into the state `execute approve` demands.
///
/// `prepare` currently stamps the board `Pending`; `execute approve`
/// requires `AwaitingApproval`. The parallel OP-EXEC-* workstream owns
/// that transition — until it lands, this pins the precondition so the
/// real approve path (the only writer of the `execution-graph` gate) can
/// run end to end.
fn awaiting_approval(root: &Utf8PathBuf) {
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
    board.set_status(ExecutionStatus::AwaitingApproval);
    board.write(&layout, &feature).unwrap();
}

fn gate(outcome: &StatusOutcome, gate: Gate) -> &GateStatus {
    outcome.gates.iter().find(|g| g.gate == gate).unwrap()
}

#[test]
fn status_shows_all_four_gates_pending_in_a_fresh_hall() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let report = status(&ctx, status_input("plans/checkout/plan.md")).unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.feature.as_str(), "checkout");
    assert_eq!(report.value.gates.len(), 4);
    for g in &report.value.gates {
        assert_eq!(g.state, GateState::Pending);
        assert!(g.invalidated_by.is_none());
    }
    assert!(report.value.board.is_none());
    assert!(!report.value.divergent);
}

/// The heart of the read surface: four gates, each with what invalidated
/// it. An edited `requirements.md` invalidates requirements by drift and
/// cascades to everything downstream.
#[test]
fn status_shows_the_four_gates_and_what_invalidated_each() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    for gate in ["requirements", "analysis", "plan"] {
        approve_gate(&ctx, gate);
    }

    // Edit requirements.md behind ivar's back.
    fs::write_text(
        &root.join("plans/checkout/requirements.md"),
        "# Requirements\n\n- [x] changed\n",
    )
    .unwrap();

    let report = status(&ctx, status_input("plans/checkout/plan.md")).unwrap();
    let gates = &report.value.gates;

    assert_eq!(
        gate(&report.value, Gate::Requirements).state,
        GateState::NeedsRevision
    );
    let reason = gate(&report.value, Gate::Requirements)
        .invalidated_by
        .as_deref()
        .expect("the drift reason must be named");
    assert!(
        reason.contains("requirements.md") && reason.contains("changed since approval"),
        "the drift reason must name the changed artifact: {reason}"
    );
    for (downstream, cascaded_from) in [
        (Gate::Analysis, "requirements"),
        (Gate::Plan, "analysis"),
        (Gate::ExecutionGraph, "plan"),
    ] {
        assert_eq!(
            gate(&report.value, downstream).state,
            GateState::NeedsRevision
        );
        let expected = format!("cascaded from `{cascaded_from}`");
        assert_eq!(
            gate(&report.value, downstream).invalidated_by.as_deref(),
            Some(expected.as_str())
        );
    }
    assert_eq!(gates.len(), 4);
}

/// Drift is only reported, never persisted: a status run must not repair
/// the approvals file behind the human's back.
#[test]
fn status_does_not_write_the_drift_it_finds() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    approve_gate(&ctx, "requirements");
    fs::write_text(
        &root.join("plans/checkout/requirements.md"),
        "# Requirements\n\n- [x] changed\n",
    )
    .unwrap();

    status(&ctx, status_input("plans/checkout/plan.md")).unwrap();

    // The stored state still says approved — status read it and left it.
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let on_disk = ApprovalState::read(&layout, &feature).unwrap().unwrap();
    assert_eq!(on_disk.state(Gate::Requirements), Some(GateState::Approved));
}

/// The board is shown next to the `execution-graph` gate, and after the
/// real approve flow the two agree: board `approved`, gate `approved`,
/// not divergent. This is the test that the two never diverge.
#[test]
fn status_shows_the_board_with_the_gate_and_they_never_diverge() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    let graph = root.join("graph.json");
    fs::write_text(&graph, GRAPH_JSON).unwrap();
    execute_prepare::prepare(
        &ctx,
        execute_prepare::PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: graph.to_string(),
        },
    )
    .unwrap();
    awaiting_approval(&root);
    execute_approve::approve(
        &ctx,
        execute_approve::ApproveInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let report = status(&ctx, status_input("plans/checkout/plan.md")).unwrap();

    let board = report.value.board.as_ref().expect("a board exists");
    assert_eq!(board.status, ExecutionStatus::Approved);
    assert_eq!(
        gate(&report.value, Gate::ExecutionGraph).state,
        GateState::Approved
    );
    assert!(!report.value.divergent);
}

/// The divergence the predecessor's TS permitted — board `approved` while
/// the gate is not — is made visible, with the way out named.
#[test]
fn status_flags_a_board_gate_divergence() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    let graph = root.join("graph.json");
    fs::write_text(&graph, GRAPH_JSON).unwrap();
    execute_prepare::prepare(
        &ctx,
        execute_prepare::PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: graph.to_string(),
        },
    )
    .unwrap();
    awaiting_approval(&root);
    execute_approve::approve(
        &ctx,
        execute_approve::ApproveInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    // Rewrite the gate to pending behind ivar's back — the TS bug state.
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let mut approvals = ApprovalState::read(&layout, &feature).unwrap().unwrap();
    approvals.set(Gate::ExecutionGraph, GateState::Pending, None);
    approvals.write(&layout, &feature).unwrap();

    let report = status(&ctx, status_input("plans/checkout/plan.md")).unwrap();

    assert!(report.value.divergent, "the divergence must be reported");
    let mut out = Vec::new();
    report.value.write_human(&mut out).unwrap();
    let rendered = String::from_utf8(out).unwrap();
    assert!(
        rendered.contains("DIVERGENCE"),
        "the human surface must make it unmissable: {rendered}"
    );
    assert!(rendered.contains("ivar feature execute approve"));
}

#[test]
fn status_accepts_the_plan_directory_as_the_plan_path() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let report = status(&ctx, status_input("plans/checkout")).unwrap();

    assert_eq!(report.value.feature.as_str(), "checkout");
}

#[test]
fn status_is_blocked_for_a_path_that_is_not_a_plan() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let failure = status(&ctx, status_input("README.md")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.status_not_a_plan");
}

/// A plan path projected through a session view dir — `plans/<feature>/...`
/// where `plans/<feature>` is a symlink to the hall's plan directory — is
/// accepted, because it resolves to the hall's real plan directory. This is
/// the path the bootstrap instructions tell an agent inside a session to run.
#[test]
fn status_accepts_a_plan_path_through_a_view_dir_symlink() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();

    // A minimal feature-session view dir: plans/checkout -> hall plans/checkout.
    let view_dir = root.join("view");
    fs::ensure_dir(&view_dir.join("plans")).unwrap();
    fs::create_symlink(&layout.plan_dir(&feature), &view_dir.join("plans/checkout")).unwrap();

    let ctx = Ctx::new(view_dir);
    let report = status(&ctx, status_input("plans/checkout/plan.md")).unwrap();

    assert_eq!(report.value.feature.as_str(), "checkout");
    assert_eq!(report.value.gates.len(), 4);
}

/// The projected path is accepted even when the plan directory does not exist
/// yet — the symlink dangles, and status still answers (all gates pending)
/// rather than refusing the path.
#[test]
fn status_accepts_a_dangling_plan_projection() {
    let (_guard, root) = hall_root();
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

    // No `plan create` was ever run: plans/checkout is a dangling symlink.
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let view_dir = root.join("view");
    fs::ensure_dir(&view_dir.join("plans")).unwrap();
    fs::create_symlink(&layout.plan_dir(&feature), &view_dir.join("plans/checkout")).unwrap();

    let ctx = Ctx::new(view_dir);
    let report = status(&ctx, status_input("plans/checkout/plan.md")).unwrap();

    assert_eq!(report.value.feature.as_str(), "checkout");
    assert!(
        report
            .value
            .gates
            .iter()
            .all(|gate| gate.state == GateState::Pending)
    );
}

/// A symlink under `plans/` that escapes to a directory outside the hall is
/// refused — canonicalisation lands on the external directory, not on a plan
/// of this hall.
#[test]
fn status_refuses_a_plan_symlink_that_escapes_the_hall() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());

    let outside = root.parent().unwrap().join("outside");
    fs::ensure_dir(&outside).unwrap();
    fs::create_symlink(&outside, &layout.root().join("plans/evil")).unwrap();

    let ctx = Ctx::new(root.clone());
    let failure = status(&ctx, status_input("plans/evil/plan.md")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.status_not_a_plan");
}

#[test]
fn the_human_surface_lists_gates_and_their_invalidation() {
    let outcome = StatusOutcome {
        root: Utf8PathBuf::from("/hall"),
        feature: FeatureName::new("checkout").unwrap(),
        plan_path: Utf8PathBuf::from("/hall/plans/checkout/plan.md"),
        gates: vec![
            GateStatus {
                gate: Gate::Requirements,
                state: GateState::NeedsRevision,
                invalidated_by: Some(
                    "`plans/checkout/requirements.md` changed since approval".to_owned(),
                ),
            },
            GateStatus {
                gate: Gate::Analysis,
                state: GateState::NeedsRevision,
                invalidated_by: Some("cascaded from `requirements`".to_owned()),
            },
            GateStatus {
                gate: Gate::Plan,
                state: GateState::Approved,
                invalidated_by: None,
            },
            GateStatus {
                gate: Gate::ExecutionGraph,
                state: GateState::Approved,
                invalidated_by: None,
            },
        ],
        board: Some(BoardStatus {
            board_path: Utf8PathBuf::from("/hall/.ivar/features/checkout/execution/board.json"),
            status: ExecutionStatus::Approved,
            workstreams: 1,
            plan_fingerprint: "abc".to_owned(),
            plan_matches: true,
        }),
        divergent: false,
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "SPDD status for feature `checkout` (plan: /hall/plans/checkout/plan.md):\n\
         \x20 requirements     needs-revision   — `plans/checkout/requirements.md` changed since approval\n\
         \x20 analysis         needs-revision   — cascaded from `requirements`\n\
         \x20 plan             approved\n\
         \x20 execution-graph  approved\n\
         Board: approved (1 workstream) — /hall/.ivar/features/checkout/execution/board.json\n"
    );
}
