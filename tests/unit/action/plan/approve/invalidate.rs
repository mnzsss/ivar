#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

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
