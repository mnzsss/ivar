//! A stuck hall sync shows in `ivar doctor` and in the session brief, read
//! from git when asked. Neither check touches the network: the tests fetch
//! themselves, and the brief and the doctor only look at the refs on disk.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::common::{git, hall_root, ivar};
use camino::{Utf8Path, Utf8PathBuf};

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

/// Commit `content` to `file` from a second clone and push it, then fetch it
/// into the hall so the hall's remote-tracking ref knows.
fn remote_commit(root: &Utf8Path, remote: &Utf8Path, file: &str, content: &str, message: &str) {
    let other = root.parent().unwrap().join("other");
    if !other.exists() {
        git(
            root.parent().unwrap(),
            &["clone", "-q", remote.as_str(), other.as_str()],
        );
        git(&other, &["config", "user.name", "other machine"]);
        git(&other, &["config", "user.email", "other@ivar.invalid"]);
        git(&other, &["config", "commit.gpgsign", "false"]);
    }
    std::fs::write(other.join(file), content).unwrap();
    git(&other, &["add", "-A"]);
    git(&other, &["commit", "-m", message]);
    git(&other, &["push", "origin", "main"]);
    git(root, &["fetch", "origin"]);
}

fn local_commit(root: &Utf8Path, file: &str, content: &str, message: &str) {
    std::fs::write(root.join(file), content).unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", message]);
}

/// Two local commits against three on the remote, none of them clashing.
fn diverged_hall() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root, remote) = hall_with_remote();
    local_commit(&root, "local-1.md", "1\n", "local 1");
    local_commit(&root, "local-2.md", "2\n", "local 2");
    for n in 1..=3 {
        remote_commit(
            &root,
            &remote,
            &format!("remote-{n}.md"),
            "r\n",
            &format!("remote {n}"),
        );
    }
    (guard, root)
}

/// A rebase onto the remote that stopped on a conflict.
fn hall_with_stopped_rebase() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root, remote) = hall_with_remote();
    local_commit(&root, "clash.md", "local\n", "local clash");
    remote_commit(&root, &remote, "clash.md", "remote\n", "remote clash");
    let status = std::process::Command::new("git")
        .args(["rebase", "origin/main"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(!status.status.success(), "the rebase should conflict");
    (guard, root)
}

fn sync_findings(root: &Utf8Path) -> Vec<serde_json::Value> {
    let output = ivar()
        .current_dir(root)
        .args(["doctor", "--json"])
        .output()
        .unwrap()
        .stdout;
    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    value["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["code"].as_str().unwrap().starts_with("hall.sync_"))
        .cloned()
        .collect()
}

fn brief(root: &Utf8Path) -> String {
    let out = ivar()
        .current_dir(root)
        .env_remove("IVAR_SESSION_ID")
        .args(["memory", "brief", "--provider", "claude-code"])
        .write_stdin(format!(
            r#"{{"hook_event_name":"SessionStart","session_id":"n1","cwd":"{root}"}}"#
        ))
        .assert()
        .success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

#[test]
fn a_diverged_hall_is_an_error_in_doctor_with_both_counts_and_the_fix() {
    let (_guard, root) = diverged_hall();

    let findings = sync_findings(&root);

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0]["code"], "hall.sync_diverged");
    let what = findings[0]["what"].as_str().unwrap();
    assert!(what.contains("main"), "{what}");
    assert!(what.contains("origin/main"), "{what}");
    assert!(what.contains("2 local commits"), "{what}");
    assert!(what.contains("3 remote commits"), "{what}");
    let fix = findings[0]["fix"].as_str().unwrap();
    assert!(fix.contains("git pull --rebase"), "{fix}");
    assert!(fix.contains("git push"), "{fix}");
}

#[test]
fn a_diverged_hall_puts_one_line_in_the_brief() {
    let (_guard, root) = diverged_hall();

    assert_eq!(
        brief(&root),
        "Hall sync blocked: main diverged from origin/main (2 local, 3 remote commits). \
         Tell the user before doing anything else and point them to ivar doctor.\n"
    );
}

#[test]
fn a_local_commit_that_was_not_pushed_is_a_warning_in_doctor_and_silent_in_the_brief() {
    let (_guard, root, _remote) = hall_with_remote();
    local_commit(&root, "page.md", "page\n", "unpushed");

    let findings = sync_findings(&root);

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0]["code"], "hall.sync_unpushed");
    assert!(
        findings[0]["what"]
            .as_str()
            .unwrap()
            .contains("1 local commit"),
        "{findings:?}"
    );
    assert_eq!(brief(&root), "");
}

#[test]
fn a_stopped_rebase_is_an_error_in_doctor_and_a_line_in_the_brief() {
    let (_guard, root) = hall_with_stopped_rebase();

    let findings = sync_findings(&root);

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0]["code"], "hall.sync_in_progress");
    assert!(
        findings[0]["what"].as_str().unwrap().contains("rebase"),
        "{findings:?}"
    );
    assert!(
        findings[0]["fix"]
            .as_str()
            .unwrap()
            .contains("git rebase --continue"),
        "{findings:?}"
    );
    let line = brief(&root);
    assert!(line.starts_with("Hall sync blocked: a rebase"), "{line}");
    assert!(line.contains("point them to ivar doctor"), "{line}");
}

#[test]
fn a_synchronised_hall_reports_nothing_and_the_brief_is_silent() {
    let (_guard, root, remote) = hall_with_remote();
    assert!(sync_findings(&root).is_empty());
    assert_eq!(brief(&root), "");

    // Behind the remote with nothing local is not stuck: a pull fixes it.
    remote_commit(&root, &remote, "page.md", "page\n", "remote");
    assert!(sync_findings(&root).is_empty());
    assert_eq!(brief(&root), "");
}

#[test]
fn the_report_clears_once_the_user_fixes_the_hall() {
    let (_guard, root) = diverged_hall();
    assert_eq!(sync_findings(&root).len(), 1);

    git(&root, &["pull", "--rebase", "origin", "main"]);
    git(&root, &["push"]);

    assert!(sync_findings(&root).is_empty());
    assert_eq!(brief(&root), "");
}
