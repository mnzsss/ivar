// tests/cli_styling.rs
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "support/integration.rs"]
mod common;

use assert_cmd::Command;
use common::{hall_root, ivar};
use predicates::prelude::*;

/// The binary, run from `cwd`, with the env vars scrubbed that would make the
/// result depend on the developer's shell: `FORCE_COLOR` and `NO_COLOR` are
/// the two `infra::term` reads, and the `IVAR_SESSION_*` bindings would make
/// `session env` resolve a real session instead of failing.
fn ivar_in(cwd: &camino::Utf8Path) -> Command {
    let mut cmd = ivar();
    cmd.current_dir(cwd)
        .env_remove("NO_COLOR")
        .env_remove("FORCE_COLOR")
        .env_remove("IVAR_SESSION_ID")
        .env_remove("IVAR_SESSION_PATH");
    cmd
}

#[test]
fn json_output_under_force_color_contains_no_ansi_escapes() {
    let (_guard, root) = hall_root();
    ivar_in(&root).arg("init").assert().success();
    ivar_in(&root)
        .env("FORCE_COLOR", "1")
        .args(["status", "--json"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| !out.contains('\x1b')))
        .stderr(predicate::function(|err: &str| !err.contains('\x1b')));
}

#[test]
fn failing_json_output_under_force_color_contains_no_ansi_escapes() {
    let (_guard, root) = hall_root();
    ivar_in(&root)
        .env("FORCE_COLOR", "1")
        .args(["feature", "status", "nonexistent-feature-xyz", "--json"])
        .assert()
        .failure()
        .stdout(predicate::function(|out: &str| !out.contains('\x1b')))
        .stderr(predicate::function(|err: &str| !err.contains('\x1b')));
}

/// Run a hook verb from `cwd` three ways — `NO_COLOR=1`, `FORCE_COLOR=1`, and
/// `FORCE_COLOR=1 --color always` — and assert all three produce the same
/// bytes with no ESC in them: a hook's output never follows the user's shell.
fn assert_hook_bytes_are_colourless(cwd: &camino::Utf8Path, args: &[&str], stdin: &str) {
    let run = |env: (&str, &str), extra: &[&str]| {
        ivar_in(cwd)
            .env(env.0, env.1)
            .args(extra)
            .args(args)
            .write_stdin(stdin)
            .output()
            .unwrap()
    };
    let plain = run(("NO_COLOR", "1"), &[]);
    for forced in [
        run(("FORCE_COLOR", "1"), &[]),
        run(("FORCE_COLOR", "1"), &["--color", "always"]),
    ] {
        assert_eq!(forced.stdout, plain.stdout);
        assert_eq!(forced.stderr, plain.stderr);
    }
    assert!(!plain.stdout.contains(&0x1b));
    assert!(!plain.stderr.contains(&0x1b));
}

#[test]
fn guard_stdin_hook_under_force_color_produces_identical_bytes_to_no_color() {
    let (_guard, root) = hall_root();
    let payload =
        r#"{"tool_name":"Write","tool_input":{"file_path":"/tmp/test.txt"},"cwd":"/tmp"}"#;
    assert_hook_bytes_are_colourless(&root, &["guard", "--provider", "claude-code"], payload);
}

#[test]
fn git_credential_hook_under_force_color_produces_identical_bytes_to_no_color() {
    let (_guard, root) = hall_root();
    assert_hook_bytes_are_colourless(&root, &["git-credential", "capability"], "");
}

/// Outside any session `session env` fails, and that failure — the bytes a
/// provider hook reads back — must stay as plain under `FORCE_COLOR` as its
/// success does.
#[test]
fn session_env_hook_failure_under_force_color_produces_identical_bytes_to_no_color() {
    let (_guard, root) = hall_root();
    assert_hook_bytes_are_colourless(&root, &["session", "env"], "");
}

#[test]
fn help_with_color_never_has_no_ansi_escapes() {
    let (_guard, root) = hall_root();
    ivar_in(&root)
        .env("FORCE_COLOR", "1")
        .args(["--color", "never", "--help"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| !out.contains('\x1b')));
}

#[test]
fn help_under_force_color_has_ansi_escapes() {
    let (_guard, root) = hall_root();
    ivar_in(&root)
        .env("FORCE_COLOR", "1")
        .args(["--help"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| out.contains('\x1b')));
}
