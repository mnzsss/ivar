#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common::{declare_repos, git, hall_root, ivar, seeded_repo};
use assert_cmd::Command;
use predicates::prelude::*;
use rstest::rstest;

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

#[rstest]
#[case::guard_stdin(
    &["guard", "--provider", "claude-code"],
    r#"{"tool_name":"Write","tool_input":{"file_path":"/tmp/test.txt"},"cwd":"/tmp"}"#
)]
#[case::git_credential(&["git-credential", "capability"], "")]
// Outside any session `session env` fails, and that failure — the bytes a
// provider hook reads back — must stay as plain under `FORCE_COLOR` as its
// success does.
#[case::session_env_failure(&["session", "env"], "")]
fn hook_bytes_under_force_color_match_no_color(#[case] args: &[&str], #[case] stdin: &str) {
    let (_guard, root) = hall_root();
    assert_hook_bytes_are_colourless(&root, args, stdin);
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

#[test]
fn colour_always_paints_the_preview_and_strips_back_to_the_plain_bytes() {
    let (_guard, root) = hall_root();
    ivar_in(&root).arg("init").assert().success();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "main");
    declare_repos(&root, &[("api", &origin, "main")]);
    ivar_in(&root).arg("sync").assert().success();
    ivar_in(&root)
        .args(["feature", "create", "checkout"])
        .assert()
        .success();
    ivar_in(&root)
        .args(["feature", "promote", "checkout", "api"])
        .assert()
        .success();
    let worktree = root.join(".ivar/repos/api/checkout");
    std::fs::write(worktree.join("work.md"), "work\n").unwrap();
    git(&worktree, &["add", "work.md"]);
    git(&worktree, &["commit", "-m", "work"]);
    let run = |extra: &[&str]| {
        let mut args = vec!["feature", "deliver", "checkout", "--preview"];
        args.extend_from_slice(extra);
        let output = ivar_in(&root)
            .args(&args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        String::from_utf8(output).expect("utf8 output")
    };

    let plain = run(&["--color", "never"]);
    let coloured = run(&["--color", "always"]);
    let json = run(&["--json", "--color", "always"]);

    assert!(
        !plain.contains('\x1b'),
        "--color never must stay plain:\n{plain}"
    );
    assert!(
        coloured.contains("\x1b["),
        "--color always must paint:\n{coloured}"
    );
    assert_eq!(anstream::adapter::strip_str(&coloured).to_string(), plain);
    assert!(!json.contains('\x1b'), "--json must stay plain:\n{json}");
}
