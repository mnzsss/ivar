#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use super::*;
use crate::action::Ctx;
use crate::domain::delivery_event::{DeliveryEvent, DeliveryScope, ObservedBy};
use crate::store::layout::Layout;

#[test]
fn a_merged_record_carries_its_merge_time() {
    let json = r#"{"url":"https://x/pull/7","number":7,"state":"MERGED","isDraft":false,
        "mergeCommit":{"oid":"deadbeef"},"headRefOid":"abc","mergedAt":"2026-11-04T10:00:00Z"}"#;
    let record: GhPrRecord = serde_json::from_str(json).unwrap();
    assert_eq!(
        PullRequest::from(record).merged_at.as_deref(),
        Some("2026-11-04T10:00:00Z")
    );
}

#[test]
fn an_open_record_has_no_merge_time() {
    for merged_at in [r#""mergedAt":null"#, r#""mergedAt":"""#, r#""x":1"#] {
        let json = format!(
            r#"{{"url":"u","number":1,"state":"OPEN","isDraft":false,"mergeCommit":null,"headRefOid":"a",{merged_at}}}"#
        );
        let record: GhPrRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(PullRequest::from(record).merged_at, None);
    }
}

#[test]
fn a_merged_pull_request_becomes_one_merged_event() {
    let pr = PullRequest {
        url: "https://x/pull/7".to_owned(),
        number: 7,
        state: "MERGED".to_owned(),
        head_oid: Some("abc".to_owned()),
        merge_commit: Some("def".to_owned()),
        is_draft: false,
        merged_at: Some("2026-11-04T10:00:00Z".to_owned()),
    };
    assert_eq!(
        merged_event(&pr, ObservedBy::Integrate),
        DeliveryEvent::Merged {
            number: 7,
            url: "https://x/pull/7".to_owned(),
            merge_commit: Some("def".to_owned()),
            merged_at: Some("2026-11-04T10:00:00Z".to_owned()),
            observed_by: ObservedBy::Integrate,
        }
    );
}

#[test]
fn the_same_merge_seen_by_two_invocations_is_the_same_event_id() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    let sink = std::sync::Arc::new(crate::test_support::RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());
    let pr = PullRequest {
        url: "https://x/pull/7".to_owned(),
        number: 7,
        state: "MERGED".to_owned(),
        head_oid: Some("abc".to_owned()),
        merge_commit: Some("def".to_owned()),
        is_draft: false,
        merged_at: None,
    };
    let scope = || DeliveryScope {
        feature: "f".into(),
        parent_feature: String::new(),
        repo: "api".into(),
        branch: "f".into(),
        head_sha: "abc".into(),
    };

    ctx.emitter(&layout)
        .emit(scope(), merged_event(&pr, ObservedBy::Cleanup));
    ctx.emitter(&layout)
        .emit(scope(), merged_event(&pr, ObservedBy::Integrate));

    let records = sink.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].event_id, records[1].event_id);
    assert_ne!(records[0].attempt_id, records[1].attempt_id);
}
