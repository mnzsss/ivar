use super::fixture::*;
use super::*;
use crate::action::memory::events::{MemoryEventScope, record_memory_event};
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
        json!({"heads": {}}),
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
