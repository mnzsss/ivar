#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use super::*;

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
