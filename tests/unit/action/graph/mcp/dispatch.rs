#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use serde_json::json;

use super::*;

#[test]
fn bounded_arg_clamps_above_the_limit() {
    let args = json!({"max_depth": 999_999_u64});
    assert_eq!(
        bounded_arg(&args, "max_depth", 5, MAX_DEPTH_LIMIT),
        MAX_DEPTH_LIMIT
    );
}

#[test]
fn bounded_arg_keeps_a_value_under_the_limit() {
    let args = json!({"max_hops": 3_u64});
    assert_eq!(bounded_arg(&args, "max_hops", 6, MAX_HOPS_LIMIT), 3);
}

#[test]
fn bounded_arg_falls_back_to_default_when_absent() {
    let args = json!({});
    assert_eq!(bounded_arg(&args, "max_depth", 5, MAX_DEPTH_LIMIT), 5);
}

#[test]
fn graph_feedback_records_a_feedback_miss_and_returns_a_plain_confirmation() {
    let db = GraphDb::open_in_memory().expect("open db");
    let args = json!({"query": "get_callers evaluateAccess", "reason": "returned zero callers for a symbol that exists"});

    let (text, count) = dispatch_tool_call(&db, None, "graph_feedback", &args, &mut |_| {
        Ok(json!({"status": "ok"}))
    })
    .expect("graph_feedback never errors the protocol response");

    assert!(text.to_lowercase().contains("recorded"), "{text}");
    assert_eq!(count, None);

    let misses = db.list_misses(&Default::default()).expect("list_misses");
    assert_eq!(misses.len(), 1);
    assert_eq!(misses[0].kind, crate::domain::graph::MissKind::Feedback);
    assert_eq!(
        misses[0].query.as_deref(),
        Some("get_callers evaluateAccess")
    );
    assert_eq!(
        misses[0].reason.as_deref(),
        Some("returned zero callers for a symbol that exists")
    );
    assert_eq!(misses[0].pattern, None);
}

#[test]
fn graph_feedback_truncates_query_and_reason_to_500_chars() {
    let db = GraphDb::open_in_memory().expect("open db");
    let long = "y".repeat(600);
    let args = json!({"query": long.clone(), "reason": long});

    dispatch_tool_call(&db, None, "graph_feedback", &args, &mut |_| Ok(json!({})))
        .expect("graph_feedback never errors the protocol response");

    let misses = db.list_misses(&Default::default()).expect("list_misses");
    assert_eq!(misses[0].query.as_ref().map(String::len), Some(500));
    assert_eq!(misses[0].reason.as_ref().map(String::len), Some(500));
}

#[test]
fn graph_feedback_missing_arguments_answers_without_recording_anything() {
    let db = GraphDb::open_in_memory().expect("open db");

    let (text, count) = dispatch_tool_call(&db, None, "graph_feedback", &json!({}), &mut |_| {
        Ok(json!({}))
    })
    .expect("graph_feedback never errors the protocol response");

    assert!(text.contains("query") && text.contains("reason"), "{text}");
    assert_eq!(count, None);
    assert_eq!(
        db.list_misses(&Default::default())
            .expect("list_misses")
            .len(),
        0
    );
}
