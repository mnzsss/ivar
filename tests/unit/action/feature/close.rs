#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::feature::create::CreateInput;
use crate::action::feature::create::create as create_action;
use crate::action::hall::{self, InitInput};
use crate::domain::feature::{RunBaseline, RunId, RunReceipt};
use crate::domain::name::{BranchName, FeatureName, RepoName, SessionId};
use crate::domain::provider::Provider;
use crate::error::Status;
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
    create_action(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    (guard, root)
}

fn close_input(outcome: &str) -> CloseInput {
    CloseInput {
        name: "checkout".to_owned(),
        outcome: outcome.to_owned(),
    }
}

#[test]
fn close_stops_sessions_preserves_execution_evidence_and_records_the_outcome() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    // A live executor session view dir and durable execution evidence.
    let sessions = root.join(".ivar/features/checkout/sessions/sess-1");
    fs::ensure_dir(&sessions).unwrap();
    fs::write_text(&sessions.join("state.json"), "{}").unwrap();
    fs::ensure_dir(&root.join(".ivar/features/checkout/execution")).unwrap();

    let report = close(&ctx, close_input("delivered")).unwrap();

    assert!(report.is_clean());
    assert!(!report.value.already_closed);
    assert_eq!(report.value.outcome, PromotionOutcome::Delivered);
    assert!(!fs::exists(&sessions).unwrap());
    assert!(fs::exists(&root.join(".ivar/features/checkout/execution")).unwrap());

    // The outcome landed in plan.md's frontmatter, body preserved.
    let layout = Layout::at(&root);
    let record = read_close(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .expect("a close record must exist");
    assert_eq!(record.outcome, "delivered");
    assert!(!record.closed_at.is_empty());
}

#[test]
fn close_refuses_an_active_run_and_preserves_sessions() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let sessions = layout.feature_sessions_dir(&feature).join("sess-1");
    fs::ensure_dir(&sessions).unwrap();
    RunReceipt::start(
        RunId::new("00000000-0000-0000-0000-000000000001").unwrap(),
        feature.clone(),
        "plans/checkout/plan.md",
        "fingerprint",
        RunBaseline::default(),
        SessionId::new("00000000-0000-0000-0000-000000000002").unwrap(),
        Provider::ClaudeCode,
        "2026-01-01T00:00:00Z",
    )
    .write(&layout)
    .unwrap();

    let failure = close(&ctx, close_input("delivered")).unwrap_err();
    assert_eq!(failure.code, "feature.close_run_active");
    assert!(fs::exists(&sessions).unwrap());
}

#[test]
fn close_is_idempotent_and_does_not_overwrite_the_recorded_outcome() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    close(&ctx, close_input("delivered")).unwrap();

    // A second close, with a different outcome, is a no-op.
    let report = close(&ctx, close_input("abandoned")).unwrap();

    assert!(report.value.already_closed);
    let layout = Layout::at(&root);
    let record = read_close(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .expect("a close record must exist");
    assert_eq!(
        record.outcome, "delivered",
        "the first recorded outcome must not be overwritten"
    );
}

#[test]
fn close_rejects_an_unknown_outcome_before_touching_anything() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let failure = close(&ctx, close_input("shipped")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.unknown_outcome");
    assert!(!fs::exists(&root.join("plans/checkout/plan.md")).unwrap());
}

#[test]
fn close_is_rejected_for_a_missing_feature() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let failure = close(
        &ctx,
        CloseInput {
            name: "ghost".to_owned(),
            outcome: "delivered".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.not_found");
}

#[test]
fn the_human_surface_names_the_outcome_and_timestamp() {
    let outcome = CloseOutcome {
        root: Utf8PathBuf::from("/hall"),
        name: FeatureName::new("checkout").unwrap(),
        outcome: PromotionOutcome::Delivered,
        closed_at: "2026-08-07T12:00:00.000000000Z".to_owned(),
        already_closed: false,
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Closed feature `checkout` (delivered) at 2026-08-07T12:00:00.000000000Z\n"
    );
}

// -- integrated outcome -----------------------------------------------------

/// A child feature — parent set, one promoted repo carrying a passing
/// receipt — seeded directly, since `create --parent` lands in a later task.
fn integrated_child_hall() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = seeded_hall();
    let layout = Layout::at(&root);

    let mut child = Feature::new(
        FeatureName::new("child").unwrap(),
        BranchName::new("child").unwrap(),
    );
    child.parent = Some(FeatureName::new("checkout").unwrap());
    child.promote(RepoName::new("api").unwrap());
    let receipt = crate::domain::feature::IntegrationReceipt {
        source_sha: "111".to_owned(),
        target_branch: BranchName::new("checkout").unwrap(),
        result_sha: "222".to_owned(),
        via: crate::domain::feature::IntegrationVia::Local,
        strategy: crate::domain::feature::IntegrationStrategy::Squash,
        pr_url: None,
        verification: crate::domain::feature::VerificationEvidence {
            command_fingerprint: "checks-v1".to_owned(),
            child: Vec::new(),
            parent: Vec::new(),
            pr_checks: Vec::new(),
            verified_at: "2026-08-14T12:00:00Z".to_owned(),
        },
    };
    child
        .promotions
        .get_mut(&RepoName::new("api").unwrap())
        .unwrap()
        .integration_receipt = Some(receipt);
    child.write(&layout).unwrap();

    (guard, root)
}

#[test]
fn closing_integrated_requires_a_child_with_passing_receipts() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    // A root (no parent) cannot close as integrated.
    let failure = close(
        &ctx,
        CloseInput {
            name: "checkout".to_owned(),
            outcome: "integrated".to_owned(),
        },
    )
    .unwrap_err();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.close_integrated_child_required");

    // A child without passing receipts cannot close as integrated either.
    let layout = Layout::at(&root);
    let mut child = Feature::new(
        FeatureName::new("child").unwrap(),
        BranchName::new("child").unwrap(),
    );
    child.parent = Some(FeatureName::new("checkout").unwrap());
    child.write(&layout).unwrap();
    let failure = close(
        &ctx,
        CloseInput {
            name: "child".to_owned(),
            outcome: "integrated".to_owned(),
        },
    )
    .unwrap_err();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.close_integrated_receipts_required");
    assert!(!fs::exists(&root.join("plans/child/plan.md")).unwrap());
}

#[test]
fn a_fresh_integrated_child_closes_as_integrated() {
    let (_guard, root) = integrated_child_hall();
    let ctx = Ctx::new(root.clone());

    let report = close(
        &ctx,
        CloseInput {
            name: "child".to_owned(),
            outcome: "integrated".to_owned(),
        },
    )
    .unwrap();

    assert!(!report.value.already_closed);
    assert_eq!(report.value.outcome, PromotionOutcome::Integrated);
    let layout = Layout::at(&root);
    let record = read_close(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .expect("a close record must exist");
    assert_eq!(record.outcome, "integrated");
}

#[test]
fn an_integrated_close_cannot_be_replaced_or_reopened() {
    let (_guard, root) = integrated_child_hall();
    let ctx = Ctx::new(root.clone());
    close(
        &ctx,
        CloseInput {
            name: "child".to_owned(),
            outcome: "integrated".to_owned(),
        },
    )
    .unwrap();

    // A second close — even with a different outcome — is a no-op.
    let report = close(
        &ctx,
        CloseInput {
            name: "child".to_owned(),
            outcome: "delivered".to_owned(),
        },
    )
    .unwrap();
    assert!(report.value.already_closed);
    let layout = Layout::at(&root);
    let record = read_close(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .expect("a close record must exist");
    assert_eq!(
        record.outcome, "integrated",
        "the integrated outcome must not be overwritten"
    );
}

/// Closing a feature stamps an outcome onto `plan.md`'s frontmatter. That is
/// ivar's own edit, not a human revising the plan, so the approval that
/// authorised the close has to survive it — otherwise closing a feature voids
/// the very gate that permitted closing it.
#[test]
fn closing_a_feature_preserves_the_plan_approval_across_its_own_stamp() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(&root);
    let feature = FeatureName::new("checkout").unwrap();

    crate::action::plan::create::create(
        &ctx,
        crate::action::plan::create::CreateInput {
            feature: "checkout".to_owned(),
            artifacts: vec![crate::action::plan::Artifact::Plan],
        },
    )
    .unwrap();
    crate::action::plan::approve::approve(
        &ctx,
        crate::action::plan::approve::ApproveInput {
            feature: "checkout".to_owned(),
            gate: "plan".to_owned(),
        },
    )
    .unwrap();

    close(&ctx, close_input("delivered")).unwrap();

    let approvals = crate::domain::feature::ApprovalState::read(&layout, &feature)
        .unwrap()
        .unwrap();
    assert_eq!(
        approvals.state(crate::domain::feature::Gate::Plan),
        Some(crate::domain::feature::GateState::Approved),
        "the close stamp must not read as a revision"
    );
    assert_eq!(
        approvals
            .record(crate::domain::feature::Gate::Plan)
            .unwrap()
            .artifact_fingerprint
            .as_deref(),
        Some(
            crate::infra::hash::file(&layout.plan_dir(&feature).join("plan.md"))
                .unwrap()
                .as_str()
        ),
        "the fingerprint must be resealed against the stamped file"
    );
}

/// Resealing an approval is not granting one. A plan a human edited before the
/// close was already drifted, and closing must not launder that away.
#[test]
fn closing_a_feature_does_not_reseal_a_plan_that_had_already_drifted() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(&root);
    let feature = FeatureName::new("checkout").unwrap();

    crate::action::plan::create::create(
        &ctx,
        crate::action::plan::create::CreateInput {
            feature: "checkout".to_owned(),
            artifacts: vec![crate::action::plan::Artifact::Plan],
        },
    )
    .unwrap();
    crate::action::plan::approve::approve(
        &ctx,
        crate::action::plan::approve::ApproveInput {
            feature: "checkout".to_owned(),
            gate: "plan".to_owned(),
        },
    )
    .unwrap();

    // A human rewrites the plan after approving it.
    let plan_path = layout.plan_dir(&feature).join("plan.md");
    let body = crate::infra::fs::read_text(&plan_path).unwrap().unwrap();
    crate::infra::fs::write_text(&plan_path, &format!("{body}\nrewritten\n")).unwrap();

    close(&ctx, close_input("delivered")).unwrap();

    let approvals = crate::domain::feature::ApprovalState::read(&layout, &feature)
        .unwrap()
        .unwrap();
    assert_ne!(
        approvals
            .record(crate::domain::feature::Gate::Plan)
            .unwrap()
            .artifact_fingerprint
            .as_deref(),
        Some(crate::infra::hash::file(&plan_path).unwrap().as_str()),
        "a plan that drifted before the close stays drifted"
    );
}
