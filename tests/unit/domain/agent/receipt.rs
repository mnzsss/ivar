#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

#[test]
fn classifies_each_terminal_outcome() {
    use AgentRunOutcome::{Blocked, Failed, Succeeded, TimedOut};
    assert_eq!(classify(Some(0), false, false, None), Succeeded);
    assert_eq!(classify(Some(1), false, false, None), Failed);
    assert_eq!(classify(Some(1), false, true, None), Blocked);
    assert_eq!(classify(Some(0), false, true, None), Blocked);
    assert_eq!(classify(None, false, true, None), Blocked);
    assert_eq!(classify(None, true, true, None), TimedOut);
    assert_eq!(classify(None, false, false, None), Failed);
}

#[test]
fn a_declared_outcome_overrides_the_exit_code_but_not_timeout_or_approval() {
    use AgentRunOutcome::{Blocked, Failed, Succeeded, TimedOut};
    assert_eq!(classify(Some(0), false, false, Some(Blocked)), Blocked);
    assert_eq!(classify(Some(0), false, false, Some(Failed)), Failed);
    assert_eq!(classify(Some(1), false, false, Some(Blocked)), Blocked);
    assert_eq!(classify(Some(0), true, false, Some(Blocked)), TimedOut);
    assert_eq!(classify(Some(0), false, true, Some(Failed)), Blocked);
    assert_eq!(classify(Some(0), false, false, Some(Succeeded)), Succeeded);
}

#[test]
fn the_last_non_empty_line_declares_the_outcome() {
    assert_eq!(
        declared_outcome("did stuff\nBLOCKED: no token\n\n  \n"),
        Some(AgentRunOutcome::Blocked)
    );
    assert_eq!(
        declared_outcome("x\n  FAILED: boom  "),
        Some(AgentRunOutcome::Failed)
    );
    assert_eq!(
        declared_outcome("BLOCKED: only line"),
        Some(AgentRunOutcome::Blocked)
    );
}

#[test]
fn a_sentinel_anywhere_but_the_last_line_is_ignored() {
    assert_eq!(declared_outcome("BLOCKED: x\nall good"), None);
    assert_eq!(
        declared_outcome("prompt: finish with BLOCKED: <error>\ndone"),
        None
    );
    assert_eq!(declared_outcome("done BLOCKED: x"), None);
    assert_eq!(declared_outcome("blocked: x"), None);
    assert_eq!(declared_outcome("**BLOCKED:** x"), None);
    assert_eq!(declared_outcome(""), None);
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
    assert_eq!(receipt.notify_summary("h").tail.len(), 2000);
}

#[test]
fn telegram_text_names_the_run_and_carries_the_tail() {
    let mut receipt =
        AgentRunReceipt::skipped("r1".into(), "nightly".into(), Trigger::Schedule, "t".into());
    receipt.outcome = AgentRunOutcome::TimedOut;
    receipt.exit_code = Some(124);
    receipt.output = "last line".into();
    assert_eq!(
        receipt.notify_summary("h").telegram_text(),
        "ivar agent nightly: timed_out\nh · schedule · r1 · exit 124\n\nlast line"
    );
}

#[test]
fn telegram_text_omits_an_absent_exit_code_and_an_empty_tail() {
    let receipt =
        AgentRunReceipt::skipped("r1".into(), "nightly".into(), Trigger::Manual, "t".into());
    assert_eq!(
        receipt.notify_summary("h").telegram_text(),
        "ivar agent nightly: skipped\nh · manual · r1"
    );
}

#[test]
fn telegram_text_fits_the_message_limit_and_keeps_the_end_of_the_output() {
    let agent = "a".repeat(100);
    let mut receipt = AgentRunReceipt::skipped(
        "20261019T090000Z-aaaaaaaa".into(),
        agent.clone(),
        Trigger::Schedule,
        "t".into(),
    );
    receipt.output = "😀".repeat(2000);
    let text = receipt.notify_summary("h").telegram_text();
    assert!(
        text.encode_utf16().count() <= 4096,
        "{}",
        text.encode_utf16().count()
    );
    assert!(text.ends_with('😀'));
    assert!(text.starts_with(&format!("ivar agent {agent}: skipped\n")));
}

fn timed(started_at: &str, ended_at: &str, exit_code: Option<i32>) -> String {
    let mut receipt = AgentRunReceipt::skipped(
        "r1".into(),
        "nightly".into(),
        Trigger::Manual,
        started_at.into(),
    );
    receipt.ended_at = ended_at.into();
    receipt.exit_code = exit_code;
    receipt.notify_summary("h").telegram_text()
}

#[test]
fn telegram_text_shows_how_long_the_run_took() {
    let text = timed(
        "2026-11-05T21:07:12.900Z",
        "2026-11-05T21:07:54.100Z",
        Some(0),
    );
    assert!(text.ends_with("\nh · manual · r1 · 42s"), "{text}");
    let text = timed("2026-11-05T21:07:12Z", "2026-11-05T21:09:26Z", Some(2));
    assert!(
        text.ends_with("\nh · manual · r1 · 2m14s · exit 2"),
        "{text}"
    );
    let text = timed("2026-11-05T23:50:00Z", "2026-11-06T01:05:30Z", None);
    assert!(text.ends_with("\nh · manual · r1 · 1h15m"), "{text}");
}

#[test]
fn telegram_text_omits_a_zero_or_unreadable_duration() {
    let instant = "2026-11-05T21:07:12Z";
    assert!(timed(instant, instant, None).ends_with("\nh · manual · r1"));
    assert!(timed("t", instant, None).ends_with("\nh · manual · r1"));
}
