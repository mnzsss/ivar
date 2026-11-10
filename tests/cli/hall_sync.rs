//! End-to-end tests for `ivar hall sync`, driving the compiled binary.
//!
//! The behaviour — which commits, which files, every conflict rule, the
//! warnings — is unit-tested in `src/action/hall/sync.rs` against real git.
//! These exist for what only the real process can prove: the exit code
//! contract (`0` clean, `1` with a warning, `2` for a failure), that `--json`
//! and the human surface report the same facts, and that the manual command
//! ignores the `auto_*` switches.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::common::{git, hall_root, ivar};
use camino::{Utf8Path, Utf8PathBuf};
use predicates::prelude::*;

/// An initialised hall on `main`, pushed to a bare remote, with one commit.
fn hall_with_remote() -> (tempfile::TempDir, Utf8PathBuf, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let remote = root.parent().unwrap().join("remote.git");
    git(
        root.parent().unwrap(),
        &[
            "init",
            "--bare",
            "--initial-branch",
            "main",
            remote.as_str(),
        ],
    );
    ivar()
        .current_dir(&root)
        .args(["init", ".", "--git"])
        .assert()
        .success();
    git(&root, &["config", "user.name", "ivar tests"]);
    git(&root, &["config", "user.email", "tests@ivar.invalid"]);
    git(&root, &["config", "commit.gpgsign", "false"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-m", "first"]);
    git(&root, &["remote", "add", "origin", remote.as_str()]);
    git(&root, &["push", "-u", "origin", "main"]);
    (guard, root, remote)
}

fn head(dir: &Utf8Path) -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn branch_tip(remote: &Utf8Path) -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "main"])
        .current_dir(remote)
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn pending_changes_are_committed_and_pushed_and_the_exit_is_zero() {
    let (_guard, root, remote) = hall_with_remote();
    std::fs::create_dir_all(root.join("docs/memory")).unwrap();
    std::fs::write(root.join("docs/memory/page.md"), "page\n").unwrap();

    ivar()
        .current_dir(&root)
        .args(["hall", "sync"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("pushed"));

    assert_eq!(head(&root), branch_tip(&remote));
}

#[test]
fn json_and_human_report_the_same_facts() {
    let (_guard, root, _remote) = hall_with_remote();
    std::fs::write(root.join("page.md"), "page\n").unwrap();

    let output = ivar()
        .current_dir(&root)
        .args(["hall", "sync", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["general_committed"], true);
    assert_eq!(value["operation_committed"], false);
    assert_eq!(value["pushed"], true);
    assert_eq!(value["root"], root.as_str());
    assert!(value.get("warnings").is_none());

    ivar()
        .current_dir(&root)
        .args(["hall", "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("nothing to sync"));
}

#[test]
fn a_hall_with_no_upstream_exits_one_with_a_stable_code() {
    let (_guard, root, _remote) = hall_with_remote();
    git(&root, &["branch", "--unset-upstream"]);
    std::fs::write(root.join("page.md"), "page\n").unwrap();

    let output = ivar()
        .current_dir(&root)
        .args(["hall", "sync", "--json"])
        .assert()
        .failure()
        .code(1)
        .get_output()
        .stdout
        .clone();

    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value["general_committed"], true);
    assert_eq!(value["pushed"], false);
    assert_eq!(value["warnings"][0]["code"], "hall.push_skipped");
}

#[test]
fn a_hook_that_refuses_exits_one_and_shows_the_hooks_own_words() {
    let (_guard, root, remote) = hall_with_remote();
    let hook = root.join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(
        &hook,
        "#!/bin/sh\necho 'page.md is over 5000 tokens' >&2\nexit 1\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let before = branch_tip(&remote);
    std::fs::write(root.join("page.md"), "page\n").unwrap();

    ivar()
        .current_dir(&root)
        .args(["hall", "sync"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("over 5000 tokens"));

    assert_eq!(branch_tip(&remote), before);
}

#[test]
fn the_manual_command_ignores_auto_commit_and_auto_push() {
    let (_guard, root, remote) = hall_with_remote();
    let path = root.join("ivar.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    manifest["auto_commit"] = serde_json::json!(false);
    manifest["auto_push"] = serde_json::json!(false);
    std::fs::write(&path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    std::fs::write(root.join("page.md"), "page\n").unwrap();

    ivar()
        .current_dir(&root)
        .args(["hall", "sync"])
        .assert()
        .success();

    assert_eq!(head(&root), branch_tip(&remote));
}

#[test]
fn syncing_outside_a_hall_exits_two_and_points_at_init() {
    let (_guard, root) = hall_root();

    ivar()
        .current_dir(&root)
        .args(["hall", "sync"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("ivar init"));
}
