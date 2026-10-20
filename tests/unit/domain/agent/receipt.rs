#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

#[test]
fn classifies_each_terminal_outcome() {
    assert_eq!(classify(Some(0), false, false), AgentRunOutcome::Succeeded);
    assert_eq!(classify(Some(1), false, false), AgentRunOutcome::Failed);
    assert_eq!(classify(Some(1), false, true), AgentRunOutcome::Blocked);
    assert_eq!(classify(Some(0), false, true), AgentRunOutcome::Blocked);
    assert_eq!(classify(None, false, true), AgentRunOutcome::Blocked);
    assert_eq!(classify(None, true, true), AgentRunOutcome::TimedOut);
    assert_eq!(classify(None, false, false), AgentRunOutcome::Failed);
}

#[test]
fn scrub_replaces_every_secret_value_and_ignores_empty_ones() {
    let text = "token=s3cr3t and again s3cr3t; other=hunter2";
    assert_eq!(
        scrub(text, &["s3cr3t", "hunter2", ""]),
        "token=[redacted] and again [redacted]; other=[redacted]"
    );
}

#[test]
fn run_id_is_sortable_by_start_time() {
    assert_eq!(
        run_id("2026-10-19T10:15:07.123456789Z", "ab12cd34"),
        "20261019T101507Z-ab12cd34"
    );
}

#[test]
fn skipped_receipt_serializes_snake_case_and_omits_absent_fields() {
    let receipt = AgentRunReceipt::skipped(
        "20261019T101507Z-ab12cd34".to_owned(),
        "nightly".to_owned(),
        Trigger::Schedule,
        "2026-10-19T10:15:07Z".to_owned(),
    );
    let json = serde_json::to_value(&receipt).unwrap();
    assert_eq!(json["outcome"], "skipped");
    assert_eq!(json["trigger"], "schedule");
    assert_eq!(json["version"], 1);
    assert!(json.get("notify_error").is_none());
    assert_eq!(receipt.started_at, receipt.ended_at);
}

#[test]
fn a_setup_failure_is_a_failed_receipt_carrying_the_reason() {
    let r = AgentRunReceipt::failed(
        "r".into(),
        "a".into(),
        Trigger::Manual,
        "t0".into(),
        "t1".into(),
        "network create failed".into(),
    );
    assert_eq!(r.outcome, AgentRunOutcome::Failed);
    assert_eq!(r.exit_code, None);
    assert_eq!(r.output, "network create failed");
    assert_eq!((r.started_at.as_str(), r.ended_at.as_str()), ("t0", "t1"));
}

#[test]
fn timed_out_serializes_with_an_underscore() {
    assert_eq!(
        serde_json::to_value(AgentRunOutcome::TimedOut).unwrap(),
        "timed_out"
    );
}

#[test]
fn notify_summary_keeps_only_the_tail() {
    let mut receipt = AgentRunReceipt::skipped("r".into(), "a".into(), Trigger::Manual, "t".into());
    receipt.output = "x".repeat(5000);
    assert_eq!(receipt.notify_summary().tail.len(), 2000);
}
