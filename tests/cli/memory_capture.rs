#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::common::ivar;

#[test]
fn capture_exits_zero_with_no_hall_and_empty_output() {
    let dir = tempfile::tempdir().unwrap();
    ivar()
        .current_dir(dir.path())
        .args(["memory", "capture", "--provider", "claude-code"])
        .write_stdin("garbage")
        .assert()
        .success()
        .stdout("");
}

#[test]
fn capture_with_an_unknown_provider_still_exits_zero() {
    ivar()
        .args(["memory", "capture", "--provider", "nope"])
        .write_stdin("{}")
        .assert()
        .success();
}

#[test]
fn capture_without_a_provider_exits_zero_with_empty_output() {
    ivar()
        .args(["memory", "capture"])
        .write_stdin("{}")
        .assert()
        .success()
        .stdout("");
}

#[test]
fn a_multi_megabyte_write_is_captured_fast_and_stays_inside_the_body_limit() {
    let (_guard, root) = crate::common::hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let payload = serde_json::json!({
        "hook_event_name": "PostToolUse",
        "session_id": "native-big",
        "tool_name": "Write",
        "tool_input": { "file_path": "big.txt", "content": "word ".repeat(600_000) },
        "tool_response": {}
    })
    .to_string();

    let started = std::time::Instant::now();
    ivar()
        .current_dir(&root)
        .env("IVAR_HALL", &root)
        .args(["memory", "capture", "--provider", "claude-code"])
        .write_stdin(payload)
        .assert()
        .success()
        .stdout("");

    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    let spool = std::fs::read_to_string(root.join(".ivar/spool/memory/native-big.jsonl")).unwrap();
    assert_eq!(spool.lines().count(), 1);
    assert!(spool.len() <= 4096 + 3 + 1024, "{} bytes", spool.len());
}

#[test]
fn a_payload_past_the_stdin_cap_is_skipped_without_failing_the_hook() {
    let (_guard, root) = crate::common::hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let payload = serde_json::json!({
        "hook_event_name": "PostToolUse",
        "session_id": "native-huge",
        "tool_name": "Write",
        "tool_input": { "content": "word ".repeat(2_000_000) },
        "tool_response": {}
    })
    .to_string();

    ivar()
        .current_dir(&root)
        .env("IVAR_HALL", &root)
        .args(["memory", "capture", "--provider", "claude-code"])
        .write_stdin(payload)
        .assert()
        .success()
        .stdout("");

    assert!(!root.join(".ivar/spool/memory/native-huge.jsonl").exists());
}
