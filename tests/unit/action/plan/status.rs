#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::approve::{self as plan_approve, ApproveInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
use crate::domain::feature::{Gate, GateState, RunBaseline, RunId, RunReceipt, RunStatus};
use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::infra::{fs, hash};
use crate::store::layout::Layout;
use crate::test_support::hall_root;

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
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    plan_create::create(
        &ctx,
        PlanCreateInput {
            feature: "checkout".to_owned(),
            artifacts: Vec::new(),
        },
    )
    .unwrap();
    (guard, root)
}

fn input() -> StatusInput {
    StatusInput {
        plan_path: "plans/checkout/plan.md".to_owned(),
    }
}

#[test]
fn status_lists_exactly_the_three_spdd_gates() {
    let (_guard, root) = seeded_hall();
    let report = status(&Ctx::new(root), input()).unwrap();
    assert_eq!(
        report
            .value
            .gates
            .iter()
            .map(|gate| gate.gate)
            .collect::<Vec<_>>(),
        Gate::ALL
    );
    assert_eq!(report.value.gates.len(), 3);
}

#[test]
fn status_reports_approval_drift_without_persisting_it() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    for gate in ["requirements", "analysis", "plan"] {
        plan_approve::approve(
            &ctx,
            ApproveInput {
                feature: "checkout".to_owned(),
                gate: gate.to_owned(),
            },
        )
        .unwrap();
    }
    fs::write_text(&root.join("plans/checkout/requirements.md"), "changed").unwrap();

    let report = status(&ctx, input()).unwrap();
    assert!(
        report
            .value
            .gates
            .iter()
            .all(|gate| gate.state == GateState::NeedsRevision)
    );
    let layout = Layout::at(root);
    let feature = FeatureName::new("checkout").unwrap();
    assert_eq!(
        ApprovalState::read(&layout, &feature)
            .unwrap()
            .unwrap()
            .record(Gate::Requirements)
            .unwrap()
            .state,
        GateState::Approved
    );
}

#[test]
fn status_projects_current_receipt_and_plan_divergence() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let plan = layout.plan_dir(&feature).join("plan.md");
    let receipt = RunReceipt::start(
        RunId::new("00000000-0000-0000-0000-000000000001").unwrap(),
        feature.clone(),
        plan.clone(),
        hash::file(&plan).unwrap(),
        RunBaseline::default(),
        SessionId::new("00000000-0000-0000-0000-000000000002").unwrap(),
        Provider::ClaudeCode,
        "2026-01-01T00:00:00Z",
    );
    receipt.write(&layout).unwrap();
    fs::write_text(&plan, "revised plan").unwrap();

    let report = status(&Ctx::new(root), input()).unwrap();
    let receipt = report.value.receipt.unwrap();
    assert_eq!(receipt.status, RunStatus::Active);
    assert!(!receipt.plan_matches);
    assert!(receipt.recovery.unwrap().contains("active"));
    assert!(report.value.evidence_available);
}

#[test]
fn status_omits_gate_whose_artifact_is_absent_and_never_approved() {
    let (_guard, root) = seeded_hall();
    fs::remove_file(&root.join("plans/checkout/requirements.md")).unwrap();

    let report = status(&Ctx::new(root), input()).unwrap();
    assert_eq!(
        report
            .value
            .gates
            .iter()
            .map(|gate| gate.gate)
            .collect::<Vec<_>>(),
        [Gate::Analysis, Gate::Plan]
    );
}

#[test]
fn status_keeps_approved_gate_as_needs_revision_when_its_artifact_is_deleted() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    plan_approve::approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "requirements".to_owned(),
        },
    )
    .unwrap();
    fs::remove_file(&root.join("plans/checkout/requirements.md")).unwrap();

    let report = status(&ctx, input()).unwrap();
    let requirements = report
        .value
        .gates
        .iter()
        .find(|gate| gate.gate == Gate::Requirements)
        .expect("an approved gate must not be omitted when its artifact vanishes");
    assert_eq!(requirements.state, GateState::NeedsRevision);
    assert!(requirements.invalidated_by.is_some());
}

#[test]
fn status_lists_all_three_gates_when_every_artifact_is_present() {
    let (_guard, root) = seeded_hall();
    let report = status(&Ctx::new(root), input()).unwrap();
    assert_eq!(
        report
            .value
            .gates
            .iter()
            .map(|gate| gate.gate)
            .collect::<Vec<_>>(),
        Gate::ALL
    );
    assert!(
        report
            .value
            .gates
            .iter()
            .all(|gate| gate.state == GateState::Pending && gate.invalidated_by.is_none())
    );
}

#[test]
fn status_refuses_a_path_outside_feature_plans() {
    let (_guard, root) = seeded_hall();
    let failure = status(
        &Ctx::new(root),
        StatusInput {
            plan_path: "README.md".to_owned(),
        },
    )
    .unwrap_err();
    assert_eq!(failure.code, "plan.status_not_a_plan");
}
