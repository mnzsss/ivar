#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
use crate::error::Status;
use crate::infra::hash;
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

/// The persisted approval state, read back off disk — the real files, not
/// the in-memory value an action returned.
fn persisted(root: &Utf8PathBuf) -> ApprovalState {
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    ApprovalState::read(&layout, &feature).unwrap().unwrap()
}

#[test]
fn approve_requirements_transitions_the_gate_to_approved() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let report = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "requirements".to_owned(),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.gate, Gate::Requirements);
    assert_eq!(
        report.value.approvals.state(Gate::Requirements),
        Some(GateState::Approved)
    );
    assert_eq!(
        report.value.approvals.state(Gate::Analysis),
        Some(GateState::Pending)
    );

    // Persisted, with the artifact's fingerprint recorded.
    let on_disk = persisted(&root);
    assert_eq!(on_disk.state(Gate::Requirements), Some(GateState::Approved));
    assert_eq!(
        on_disk
            .record(Gate::Requirements)
            .unwrap()
            .artifact_fingerprint,
        Some(hash::file(&root.join("plans/checkout/requirements.md")).unwrap())
    );
}

#[test]
fn approve_execution_graph_is_an_unknown_gate() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let failure = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "execution-graph".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.unknown_gate");
}

#[test]
fn approve_analysis_requires_requirements_approved() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let failure = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "analysis".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.upstream_not_approved");
    assert_eq!(failure.fix_actions.len(), 1);
    assert!(failure.fix_actions[0].safe);

    // Approving the upstream gate first unblocks it.
    approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "requirements".to_owned(),
        },
    )
    .unwrap();
    let report = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "analysis".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(
        report.value.approvals.state(Gate::Analysis),
        Some(GateState::Approved)
    );
}

#[test]
fn approve_plan_succeeds_when_both_upstream_artifacts_are_absent() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    // A light feature: only plan.md is meant to exist. Delete the other two
    // scaffolded artifacts behind ivar's back to model that.
    fs::remove_path(&root.join("plans/checkout/requirements.md")).unwrap();
    fs::remove_path(&root.join("plans/checkout/analysis.md")).unwrap();

    let report = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "plan".to_owned(),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.gate, Gate::Plan);
    assert_eq!(
        report.value.approvals.state(Gate::Plan),
        Some(GateState::Approved)
    );

    let on_disk = persisted(&root);
    assert_eq!(on_disk.state(Gate::Plan), Some(GateState::Approved));
}

#[test]
fn approve_plan_blocked_by_written_unapproved_requirements_when_analysis_absent() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    // requirements.md exists and is unapproved (still Pending); analysis.md is
    // absent. An immediate-upstream-only check would see analysis.md missing
    // and wave `plan` straight through — that is exactly the regression this
    // rule must not reintroduce (F1). The escape is "never written", never
    // "written and ignored".
    fs::remove_path(&root.join("plans/checkout/analysis.md")).unwrap();

    let failure = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "plan".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.upstream_not_approved");
    assert!(
        failure.what.contains("requirements"),
        "failure should name `requirements`, the actual blocking gate: {}",
        failure.what
    );
    assert!(
        !failure.what.contains("analysis"),
        "an absent analysis.md must never be named as blocking: {}",
        failure.what
    );
}

#[test]
fn approve_plan_succeeds_when_requirements_is_approved_and_analysis_is_absent() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "requirements".to_owned(),
        },
    )
    .unwrap();
    fs::remove_path(&root.join("plans/checkout/analysis.md")).unwrap();

    let report = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "plan".to_owned(),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert_eq!(
        report.value.approvals.state(Gate::Plan),
        Some(GateState::Approved)
    );
}

#[test]
fn approving_all_three_gates_in_order_behaves_exactly_as_before_this_change() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    for gate in ["requirements", "analysis", "plan"] {
        let report = approve(
            &ctx,
            ApproveInput {
                feature: "checkout".to_owned(),
                gate: gate.to_owned(),
            },
        )
        .unwrap();
        assert!(report.is_clean());
    }

    let on_disk = persisted(&root);
    for gate in Gate::ALL {
        assert_eq!(on_disk.state(gate), Some(GateState::Approved));
    }
}

#[test]
fn deleting_an_approved_requirements_artifact_still_cascades_to_needs_revision() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    for gate in ["requirements", "analysis", "plan"] {
        approve(
            &ctx,
            ApproveInput {
                feature: "checkout".to_owned(),
                gate: gate.to_owned(),
            },
        )
        .unwrap();
    }

    // The approved artifact vanishes entirely, rather than merely changing
    // content.
    fs::remove_path(&root.join("plans/checkout/requirements.md")).unwrap();

    // Reconcile runs — and persists — before any refusal, so the next
    // approval attempt still records the honest, cascaded state even though
    // it is itself refused.
    let failure = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "plan".to_owned(),
        },
    )
    .unwrap_err();
    assert_eq!(failure.code, "plan.upstream_not_approved");

    // Absence must not silently clear an approval that was already crossed:
    // every gate downstream of (and including) the vanished artifact is
    // `needs-revision`, never omitted and never left `approved`.
    let on_disk = persisted(&root);
    for gate in Gate::ALL {
        assert_eq!(
            on_disk.state(gate),
            Some(GateState::NeedsRevision),
            "{gate} should need revision after requirements.md was deleted"
        );
    }
}

#[test]
fn an_approved_gate_blocks_edits_to_its_artifact_detected_by_fingerprint_change() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "requirements".to_owned(),
        },
    )
    .unwrap();

    // Edit requirements.md behind ivar's back.
    fs::write_text(
        &root.join("plans/checkout/requirements.md"),
        "# Requirements\n\n- [x] changed\n",
    )
    .unwrap();

    // The next approval attempt refuses: requirements is no longer approved.
    let failure = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "analysis".to_owned(),
        },
    )
    .unwrap_err();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.upstream_not_approved");

    // And the state honestly records the drift: the edited gate and
    // everything downstream of it need revision.
    let on_disk = persisted(&root);
    assert_eq!(
        on_disk.state(Gate::Requirements),
        Some(GateState::NeedsRevision)
    );
    assert_eq!(
        on_disk.state(Gate::Analysis),
        Some(GateState::NeedsRevision)
    );
}

#[test]
fn upstream_invalidation_cascades_to_every_downstream_gate() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    for gate in ["requirements", "analysis", "plan"] {
        approve(
            &ctx,
            ApproveInput {
                feature: "checkout".to_owned(),
                gate: gate.to_owned(),
            },
        )
        .unwrap();
    }

    fs::write_text(
        &root.join("plans/checkout/requirements.md"),
        "# Requirements\n\n- [x] changed\n",
    )
    .unwrap();

    // Any approval attempt reconciles first, finds the drift, and refuses.
    let failure = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "plan".to_owned(),
        },
    )
    .unwrap_err();
    assert_eq!(failure.code, "plan.upstream_not_approved");

    let on_disk = persisted(&root);
    for gate in Gate::ALL {
        assert_eq!(
            on_disk.state(gate),
            Some(GateState::NeedsRevision),
            "{gate} should need revision after requirements changed"
        );
    }
}

#[test]
fn reapproving_after_a_fix_transitions_back_to_approved() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "requirements".to_owned(),
        },
    )
    .unwrap();
    approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "analysis".to_owned(),
        },
    )
    .unwrap();

    // Drift: requirements.md changes.
    fs::write_text(
        &root.join("plans/checkout/requirements.md"),
        "# Requirements\n\n- [x] changed\n",
    )
    .unwrap();
    let failure = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "plan".to_owned(),
        },
    )
    .unwrap_err();
    assert_eq!(failure.code, "plan.upstream_not_approved");

    // The human reviews the new requirements and re-approves them; the
    // downstream gates follow.
    approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "requirements".to_owned(),
        },
    )
    .unwrap();
    let report = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "analysis".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(
        report.value.approvals.state(Gate::Requirements),
        Some(GateState::Approved)
    );
    assert_eq!(
        report.value.approvals.state(Gate::Analysis),
        Some(GateState::Approved)
    );
}

#[test]
fn approve_is_blocked_when_the_gates_artifact_is_missing() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    // plan.md was scaffolded by `plan create`; delete it behind ivar's back.
    fs::remove_path(&root.join("plans/checkout/plan.md")).unwrap();

    let failure = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "plan".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.artifact_missing");
}

#[test]
fn approve_is_blocked_for_an_unknown_gate() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let failure = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "bogus".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.unknown_gate");
}

#[test]
fn approve_is_blocked_for_a_missing_feature() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let failure = approve(
        &ctx,
        ApproveInput {
            feature: "ghost".to_owned(),
            gate: "requirements".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.feature_not_found");
}

#[test]
fn invalidate_marks_the_gate_and_downstream_needs_revision() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    for gate in ["requirements", "analysis"] {
        approve(
            &ctx,
            ApproveInput {
                feature: "checkout".to_owned(),
                gate: gate.to_owned(),
            },
        )
        .unwrap();
    }

    let report = invalidate(
        &ctx,
        InvalidateInput {
            feature: "checkout".to_owned(),
            gate: "requirements".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(report.value.gate, Gate::Requirements);
    assert_eq!(report.value.cascaded.len(), 3);
    for gate in Gate::ALL {
        assert_eq!(
            report.value.approvals.state(gate),
            Some(GateState::NeedsRevision)
        );
    }

    // The fingerprints are gone from disk too — an invalidated approval
    // is void, with nothing left to compare against.
    let on_disk = persisted(&root);
    for gate in Gate::ALL {
        assert_eq!(on_disk.record(gate).unwrap().artifact_fingerprint, None);
    }
}

#[test]
fn invalidate_is_idempotent() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "requirements".to_owned(),
        },
    )
    .unwrap();

    let first = invalidate(
        &ctx,
        InvalidateInput {
            feature: "checkout".to_owned(),
            gate: "requirements".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(first.value.cascaded.len(), 3);

    let second = invalidate(
        &ctx,
        InvalidateInput {
            feature: "checkout".to_owned(),
            gate: "requirements".to_owned(),
        },
    )
    .unwrap();

    assert!(second.value.cascaded.is_empty());
    assert_eq!(first.value.approvals, second.value.approvals);
}

#[test]
fn invalidate_is_blocked_for_a_missing_feature() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let failure = invalidate(
        &ctx,
        InvalidateInput {
            feature: "ghost".to_owned(),
            gate: "requirements".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.feature_not_found");
}

#[test]
fn the_approve_human_surface_lists_every_gate_state() {
    let mut approvals = ApprovalState::fresh();
    approvals.set(Gate::Requirements, GateState::Approved, None);
    let outcome = ApproveOutcome {
        root: Utf8PathBuf::from("/hall"),
        feature: FeatureName::new("checkout").unwrap(),
        gate: Gate::Requirements,
        approvals,
        visible: Gate::ALL.to_vec(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Approved `requirements` for feature `checkout`\n\
         \x20 requirements     approved\n\
         \x20 analysis         pending\n\
         \x20 plan             pending\n"
    );
}

#[test]
fn the_invalidate_human_surface_lists_every_gate_state() {
    let mut approvals = ApprovalState::fresh();
    for gate in Gate::ALL {
        approvals.set(gate, GateState::NeedsRevision, None);
    }
    let outcome = InvalidateOutcome {
        root: Utf8PathBuf::from("/hall"),
        feature: FeatureName::new("checkout").unwrap(),
        gate: Gate::Requirements,
        cascaded: Gate::ALL.to_vec(),
        approvals,
        visible: Gate::ALL.to_vec(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Invalidated `requirements` for feature `checkout`\n\
         \x20 requirements     needs-revision\n\
         \x20 analysis         needs-revision\n\
         \x20 plan             needs-revision\n"
    );
}

/// A feature that took the short path has one gate, and the approve surface
/// must say so. Rendering the whole `ApprovalState` would announce
/// `requirements pending` for an artifact nobody wrote, which is exactly the
/// instruction the short path exists to stop giving.
#[test]
fn the_approve_human_surface_omits_gates_the_feature_does_not_have() {
    let mut approvals = ApprovalState::fresh();
    approvals.set(Gate::Plan, GateState::Approved, None);
    let outcome = ApproveOutcome {
        root: Utf8PathBuf::from("/hall"),
        feature: FeatureName::new("checkout").unwrap(),
        gate: Gate::Plan,
        approvals,
        visible: vec![Gate::Plan],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Approved `plan` for feature `checkout`\n\
         \x20 plan             approved\n"
    );
}
