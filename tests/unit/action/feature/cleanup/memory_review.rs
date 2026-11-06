use super::*;
use crate::action::feature::cleanup::CleanupOutcome;
use crate::action::memory::events::{MemoryEventScope, record_memory_event};
use crate::domain::delivery_event::DeliveryEvent;
use crate::domain::observation::SourceEvent;
use crate::error::Report;
use crate::test_support::RecordingSink;
use std::sync::Arc;

fn review(root: &Utf8PathBuf, event: SourceEvent) {
    let layout = Layout::at(root.clone());
    let scope = MemoryEventScope {
        session_id: "s1".to_owned(),
        feature: "checkout".to_owned(),
        parent_feature: String::new(),
    };
    record_memory_event(
        &layout,
        &scope,
        event,
        &serde_json::json!({"heads": {}, "reason": "hotfix"}),
    )
    .unwrap();
    crate::action::memory::drain::drain(&layout).unwrap();
}

fn not_reviewed(warnings: &[crate::error::Warning]) -> bool {
    warnings
        .iter()
        .any(|w| w.code == "cleanup.memory_not_reviewed" && w.subject == "checkout")
}

fn preview_report(root: &Utf8PathBuf) -> Report<CleanupOutcome> {
    cleanup(
        &Ctx::new(root.clone()),
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: true,
            record: None,
            session_id: None,
        },
    )
    .unwrap()
}

fn apply_schema_two(ctx: &Ctx, root: &Utf8PathBuf) -> Report<CleanupOutcome> {
    let fingerprint = preview_report(root).value.preview.fingerprint;
    let layout = Layout::at(root.clone());
    fs::ensure_dir(&layout.update_pages_dir()).unwrap();
    let path = Utf8PathBuf::from("docs/memory/update/001-checkout.cleanup.json");
    let record = serde_json::json!({
        "schema_version": 2,
        "feature": "checkout",
        "branch": "checkout",
        "fingerprint": fingerprint,
        "approvals": {
            "delivery": { "approved": true, "at": "2026-11-06T12:00:00Z" },
            "teardown": { "approved": true, "at": "2026-11-06T12:10:00Z" }
        },
        "outcome": null
    });
    fs::write_text(&root.join(&path), &record.to_string()).unwrap();
    cleanup(
        ctx,
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: false,
            record: Some(path),
            session_id: None,
        },
    )
    .unwrap()
}

fn cleaned_docs(sink: &RecordingSink) -> Vec<String> {
    sink.records()
        .iter()
        .filter_map(|r| match &r.event {
            DeliveryEvent::Cleaned { docs, .. } => Some(docs.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn without_a_review_the_preview_and_the_apply_warn_and_docs_is_missing() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    assert!(not_reviewed(&preview_report(&root).warnings));

    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root.clone()).with_delivery_events(sink.clone());
    let applied = apply_schema_two(&ctx, &root);

    assert!(
        applied
            .value
            .apply_outcome
            .as_ref()
            .is_some_and(|o| o.feature_removed)
    );
    assert!(not_reviewed(&applied.warnings));
    assert_eq!(cleaned_docs(&sink), ["missing"]);
}

#[test]
fn a_reviewed_feature_cleans_up_without_the_warning_and_docs_is_reviewed() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    review(&root, SourceEvent::MemoryReviewed);
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root.clone()).with_delivery_events(sink.clone());

    let applied = apply_schema_two(&ctx, &root);

    assert!(!not_reviewed(&applied.warnings), "{:?}", applied.warnings);
    assert_eq!(cleaned_docs(&sink), ["reviewed"]);
}

#[test]
fn a_skipped_review_counts_as_checked_and_docs_is_skipped() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    review(&root, SourceEvent::MemoryReviewSkipped);
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root.clone()).with_delivery_events(sink.clone());

    let applied = apply_schema_two(&ctx, &root);

    assert!(!not_reviewed(&applied.warnings), "{:?}", applied.warnings);
    assert_eq!(cleaned_docs(&sink), ["skipped"]);
}
