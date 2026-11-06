use super::fixture::*;
use super::*;
use crate::action::memory::events::{MemoryEventScope, record_memory_event};
use crate::action::memory::review::{ReviewState, review_state};
use crate::domain::feature::MemoryReviewGate;
use crate::domain::observation::SourceEvent;
use serde_json::json;

fn preview(root: &Utf8Path) -> Report<DeliverOutcome> {
    deliver(&Ctx::new(root.to_path_buf()), preview_input("checkout")).unwrap()
}

fn apply(root: &Utf8Path, fingerprint: &str) -> Failure {
    deliver(
        &Ctx::new(root.to_path_buf()),
        apply_input("checkout", fingerprint),
    )
    .unwrap_err()
}

#[test]
fn without_a_review_the_preview_lists_the_blocker_and_apply_refuses() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_plan_gates(&root);

    let report = preview(&root);
    assert_eq!(
        report.value.preview.memory_review,
        MemoryReviewGate::Missing
    );
    assert!(
        report
            .value
            .blockers
            .iter()
            .any(|b| b.contains("memory review is missing")),
        "{:?}",
        report.value.blockers
    );

    let failure = apply(&root, &report.value.preview.fingerprint);
    assert_eq!(failure.code, "deliver.memory_not_reviewed");
    assert!(
        failure.fix_actions[0].what.contains("memory_review"),
        "{failure:?}"
    );
}

#[test]
fn a_review_recorded_after_the_preview_drifts_the_fingerprint() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_plan_gates(&root);
    let stale = preview(&root).value.preview.fingerprint;

    record_memory_review(&root, "checkout", json!({}));

    assert_eq!(apply(&root, &stale).code, "deliver.fingerprint_mismatch");
}

#[test]
fn a_review_still_in_the_spool_is_drained_before_the_gate() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_plan_gates(&root);
    let scope = MemoryEventScope {
        session_id: "s1".to_owned(),
        feature: "checkout".to_owned(),
        parent_feature: String::new(),
    };
    record_memory_event(
        &Layout::at(root.clone()),
        &scope,
        SourceEvent::MemoryReviewed,
        &json!({"heads": {}}),
    )
    .unwrap();

    let report = preview(&root);

    assert_eq!(
        report.value.preview.memory_review,
        MemoryReviewGate::Reviewed
    );
    assert!(
        report.value.blockers.is_empty(),
        "{:?}",
        report.value.blockers
    );
}

#[test]
fn a_head_moved_since_the_review_warns_without_blocking() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_plan_gates(&root);
    record_memory_review(
        &root,
        "checkout",
        json!({"api": "0000000000000000000000000000000000000000"}),
    );

    let report = preview(&root);

    assert_eq!(
        report.value.preview.memory_review,
        MemoryReviewGate::Reviewed
    );
    assert!(
        report.value.blockers.is_empty(),
        "{:?}",
        report.value.blockers
    );
    let outdated: Vec<&str> = report
        .warnings
        .iter()
        .filter(|w| w.code == "deliver.memory_review_outdated")
        .map(|w| w.subject.as_str())
        .collect();
    assert_eq!(outdated, ["api"]);
}

#[test]
fn a_skip_alone_does_not_satisfy_the_gate() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_plan_gates(&root);
    record_memory(
        &root,
        "checkout",
        SourceEvent::MemoryReviewSkipped,
        json!({"reason": "hotfix"}),
    );

    assert_eq!(
        preview(&root).value.preview.memory_review,
        MemoryReviewGate::Missing
    );
}

fn apply_skipping(
    root: &Utf8Path,
    fingerprint: &str,
    reason: &str,
) -> Result<Report<DeliverOutcome>, Failure> {
    deliver(
        &Ctx::new(root.to_path_buf()),
        DeliverInput {
            skip_memory_review: Some(reason.to_owned()),
            ..apply_input("checkout", fingerprint)
        },
    )
}

fn skip_state(root: &Utf8Path) -> ReviewState {
    let layout = Layout::at(root.to_path_buf());
    crate::action::memory::drain::drain(&layout).unwrap();
    review_state(&layout, &FeatureName::new("checkout").unwrap()).unwrap()
}

#[test]
fn a_skip_with_a_reason_lifts_only_the_memory_gate_and_records_the_skip() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_plan_gates(&root);
    let fingerprint = preview(&root).value.preview.fingerprint;

    let report = apply_skipping(&root, &fingerprint, "hotfix for prod").unwrap();

    assert_eq!(
        report.value.preview.memory_review,
        MemoryReviewGate::Missing
    );
    assert!(!report.value.pushes.is_empty(), "{:?}", report.value);
    assert_eq!(skip_state(&root), ReviewState::Skipped);
}

#[test]
fn a_skip_still_refuses_every_other_blocker_and_records_nothing() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_plan_gates(&root);
    std::fs::write(root.join(".ivar/repos/api/checkout/wip.md"), "wip\n").unwrap();
    let fingerprint = preview(&root).value.preview.fingerprint;

    let failure = apply_skipping(&root, &fingerprint, "hotfix").unwrap_err();

    assert_eq!(failure.code, "deliver.blocked");
    let actual = failure.actual.clone().unwrap_or_default();
    assert!(actual.contains("uncommitted"), "{actual}");
    assert!(!actual.contains("memory review"), "{actual}");
    assert_eq!(skip_state(&root), ReviewState::Missing);
}

#[test]
fn a_skip_is_refused_with_the_preview_and_with_an_empty_reason() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_plan_gates(&root);
    let ctx = Ctx::new(root.clone());

    let with_preview = deliver(
        &ctx,
        DeliverInput {
            skip_memory_review: Some("hotfix".to_owned()),
            ..preview_input("checkout")
        },
    )
    .unwrap_err();
    let fingerprint = preview(&root).value.preview.fingerprint;
    let blank = apply_skipping(&root, &fingerprint, "   ").unwrap_err();

    assert_eq!(with_preview.code, "deliver.skip_memory_review_with_preview");
    assert_eq!(blank.code, "deliver.skip_memory_review_reason_required");
}
