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
