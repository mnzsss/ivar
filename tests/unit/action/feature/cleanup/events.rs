use super::*;
use crate::domain::delivery_event::DeliveryEvent;
use crate::test_support::RecordingSink;
use std::sync::Arc;

#[test]
fn a_completed_cleanup_emits_one_cleaned_event_with_the_record_and_the_docs_decision() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root.clone()).with_delivery_events(sink.clone());

    let outcome = apply_with_approved_record_in(&ctx, &root);

    assert!(outcome.feature_removed);
    let records = sink.records();
    let cleaned: Vec<_> = records
        .iter()
        .filter(|r| r.event.source_event().as_str() == "delivery.cleaned")
        .collect();
    assert_eq!(cleaned.len(), 1);
    assert_eq!(cleaned[0].scope.feature, "checkout");
    match &cleaned[0].event {
        DeliveryEvent::Cleaned { record_path, docs } => {
            assert_eq!(record_path, "docs/updates/001-checkout.cleanup.json");
            assert_eq!(docs, "written");
        }
        other => panic!("expected cleaned, got {other:?}"),
    }
}

#[test]
fn a_cleanup_preview_emits_no_cleaned_event() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());

    cleanup(
        &ctx,
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: true,
            record: None,
            session_id: None,
        },
    )
    .unwrap();

    assert!(
        sink.records()
            .iter()
            .all(|r| r.event.source_event().as_str() != "delivery.cleaned")
    );
}
