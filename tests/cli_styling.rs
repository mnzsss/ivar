// tests/cli_styling.rs
#![allow(clippy::unwrap_used, clippy::expect_used)]

use assert_cmd::Command;
use predicates::prelude::*;

/// The env vars a test must scrub to get a deterministic colour decision regardless
/// of the developer's shell. `FORCE_COLOR` and `NO_COLOR` are the two that the
/// `infra::term` precedence reads; anything else is incidental.
fn scrub_colour_env(cmd: &mut Command) {
    cmd.env_remove("NO_COLOR")
        .env_remove("FORCE_COLOR")
        .env_remove("IVAR_SESSION_ID")
        .env_remove("IVAR_FEATURE")
        .env_remove("IVAR_SESSION_PATH");
}

#[test]
fn json_output_under_force_color_contains_no_ansi_escapes() {
    let mut cmd = Command::cargo_bin("ivar").unwrap();
    scrub_colour_env(&mut cmd);
    cmd.env("FORCE_COLOR", "1")
        .args(["status", "--json"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| !out.contains('\x1b')))
        .stderr(predicate::function(|err: &str| !err.contains('\x1b')));
}

#[test]
fn failing_json_output_under_force_color_contains_no_ansi_escapes() {
    let mut cmd = Command::cargo_bin("ivar").unwrap();
    scrub_colour_env(&mut cmd);
    cmd.env("FORCE_COLOR", "1")
        .args(["feature", "status", "nonexistent-feature-xyz", "--json"])
        .assert()
        .failure()
        .stdout(predicate::function(|out: &str| !out.contains('\x1b')))
        .stderr(predicate::function(|err: &str| !err.contains('\x1b')));
}
#[test]
fn guard_stdin_hook_under_force_color_produces_identical_bytes_to_no_color() {
    let payload =
        r#"{"tool_name":"Write","tool_input":{"file_path":"/tmp/test.txt"},"cwd":"/tmp"}"#;

    let mut cmd_force = Command::cargo_bin("ivar").unwrap();
    scrub_colour_env(&mut cmd_force);
    let force_out = cmd_force
        .env("FORCE_COLOR", "1")
        .args(["guard", "--provider", "claude-code"])
        .write_stdin(payload)
        .output()
        .unwrap();

    let mut cmd_no_color = Command::cargo_bin("ivar").unwrap();
    scrub_colour_env(&mut cmd_no_color);
    let no_color_out = cmd_no_color
        .env("NO_COLOR", "1")
        .args(["guard", "--provider", "claude-code"])
        .write_stdin(payload)
        .output()
        .unwrap();

    assert_eq!(force_out.stdout, no_color_out.stdout);
    assert_eq!(force_out.stderr, no_color_out.stderr);
    assert!(!force_out.stdout.contains(&0x1b));
    assert!(!force_out.stderr.contains(&0x1b));
}

#[test]
fn git_credential_hook_under_force_color_produces_identical_bytes_to_no_color() {
    let mut cmd_force = Command::cargo_bin("ivar").unwrap();
    scrub_colour_env(&mut cmd_force);
    let force_out = cmd_force
        .env("FORCE_COLOR", "1")
        .args(["git-credential", "capability"])
        .output()
        .unwrap();

    let mut cmd_no_color = Command::cargo_bin("ivar").unwrap();
    scrub_colour_env(&mut cmd_no_color);
    let no_color_out = cmd_no_color
        .env("NO_COLOR", "1")
        .args(["git-credential", "capability"])
        .output()
        .unwrap();

    assert_eq!(force_out.stdout, no_color_out.stdout);
    assert_eq!(force_out.stderr, no_color_out.stderr);
    assert!(!force_out.stdout.contains(&0x1b));
    assert!(!force_out.stderr.contains(&0x1b));
}

#[test]
fn session_env_hook_under_force_color_produces_identical_bytes_to_no_color() {
    let mut cmd_force = Command::cargo_bin("ivar").unwrap();
    scrub_colour_env(&mut cmd_force);
    let force_out = cmd_force
        .env("FORCE_COLOR", "1")
        .args(["session", "env"])
        .output()
        .unwrap();

    let mut cmd_no_color = Command::cargo_bin("ivar").unwrap();
    scrub_colour_env(&mut cmd_no_color);
    let no_color_out = cmd_no_color
        .env("NO_COLOR", "1")
        .args(["session", "env"])
        .output()
        .unwrap();

    assert_eq!(force_out.stdout, no_color_out.stdout);
    assert_eq!(force_out.stderr, no_color_out.stderr);
    assert!(!force_out.stdout.contains(&0x1b));
    assert!(!force_out.stderr.contains(&0x1b));
}

#[test]
fn help_with_color_never_has_no_ansi_escapes() {
    let mut cmd = Command::cargo_bin("ivar").unwrap();
    scrub_colour_env(&mut cmd);
    cmd.env("FORCE_COLOR", "1")
        .args(["--color", "never", "--help"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| !out.contains('\x1b')));
}

#[test]
fn help_under_force_color_has_ansi_escapes() {
    let mut cmd = Command::cargo_bin("ivar").unwrap();
    scrub_colour_env(&mut cmd);
    cmd.env("FORCE_COLOR", "1")
        .args(["--help"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| out.contains('\x1b')));
}
