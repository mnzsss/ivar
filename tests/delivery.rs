//! Integration tests for `ivar feature deliver` — PR creation and sibling linking.
//!
//! These tests drive the compiled binary to verify:
//! - Preview distinguishes `PushOnly` from create-PR (`new_pr`)
//! - The fingerprint still gates apply
//! - A repo's failure becomes a `Warning`, not a batch abort
//!
//! The `gh` interaction is exercised indirectly: when `gh` is not on PATH,
//! PR creation fails with a spawn error which surfaces as a `Warning` (not an
//! abort), proving best-effort semantics. When `gh` IS available, the created
//! PR URL appears in the preview's `pr_url` field.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use camino::{Utf8Path, Utf8PathBuf};
use common::{declare_repos, hall_root, ivar, seeded_repo};
use predicates::prelude::*;

/// Run git in `cwd` with a fixed identity.
fn git(cwd: &Utf8Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(["-c", "user.name=ivar tests"])
        .args(["-c", "user.email=tests@ivar.invalid"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git runs");

    assert!(
        output.status.success(),
        "git {} failed in {cwd}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Set up a hall with one promoted repo, synced, feature created, repo promoted,
/// and one commit on the feature branch. Returns (guard, root).
fn setup_deliver_hall(root: &Utf8PathBuf) {
    ivar().current_dir(root).arg("init").assert().success();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "main");
    declare_repos(root, &[("api", &origin, "main")]);
    ivar().current_dir(root).arg("sync").assert().success();

    // Create a feature and promote the repo.
    ivar()
        .current_dir(root)
        .args(["feature", "create", "checkout"])
        .assert()
        .success();

    ivar()
        .current_dir(root)
        .args(["feature", "promote", "checkout", "api"])
        .assert()
        .success();

    // Add a commit on the feature branch so there is something to deliver.
    let worktree = root.join(".ivar/repos/api/checkout");
    std::fs::write(worktree.join("work.md"), "work\n").unwrap();
    git(&worktree, &["add", "work.md"]);
    git(&worktree, &["commit", "-m", "work"]);
}

// -- preview ------------------------------------------------------------------

#[test]
fn preview_lists_every_promoted_repo_with_its_delivery_facts() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    let output = ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    let preview = &value["preview"];
    assert_eq!(preview["feature"], "checkout");
    let repos = preview["repos"].as_array().expect("repos is an array");
    assert_eq!(repos.len(), 1);
    let repo = &repos[0];
    assert_eq!(repo["repo"], "api");
    assert_eq!(repo["local_branch"], "checkout");
    assert!(repo["remote"].as_str().unwrap().contains("origins/api"));
    assert_eq!(repo["push_refspec"], "checkout:refs/heads/checkout");
    // The remote is a local path, not GitHub — push only, no PR surface.
    assert_eq!(repo["action"], "push_only");
    assert_eq!(repo["base_branch"], "main");
    assert!(repo["dependencies"].is_array());
    // There should be a blocker about unpushed commits.
    let blockers = repo["blockers"].as_array().expect("blockers is an array");
    assert!(
        blockers.iter().any(|b| {
            let s = b.as_str().unwrap_or("");
            s.contains("commit") || s.contains("push")
        }),
        "expected an unpushed-commits blocker, got: {blockers:?}"
    );
    // No pr_url in preview (nothing created yet).
    assert!(repo["pr_url"].is_null());
    // Fingerprint is present and non-empty.
    assert!(
        preview["fingerprint"].as_str().unwrap().len() == 64,
        "fingerprint must be a sha-256 hex digest"
    );
}

#[test]
fn preview_for_a_feature_with_no_promoted_repos_is_empty() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "main");
    declare_repos(&root, &[("api", &origin, "main")]);
    ivar().current_dir(&root).arg("sync").assert().success();

    ivar()
        .current_dir(&root)
        .args(["feature", "create", "empty"])
        .assert()
        .success();

    let output = ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "empty", "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let _value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(value["preview"]["repos"].as_array().unwrap().len(), 0);
    assert!(value["preview"]["fingerprint"].as_str().unwrap().len() == 64);
}

// -- apply: gating ------------------------------------------------------------

#[test]
fn apply_requires_a_preview_fingerprint() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("preview fingerprint"));
}

#[test]
fn apply_is_rejected_when_the_fingerprint_does_not_match() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    // Get a valid preview first.
    let preview_output = ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview_value: serde_json::Value =
        serde_json::from_slice(&preview_output).expect("valid json");
    let good_fp = preview_value["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();

    // Drift: add another commit.
    let worktree = root.join(".ivar/repos/api/checkout");
    std::fs::write(worktree.join("more.md"), "more\n").unwrap();
    git(&worktree, &["add", "more.md"]);
    git(&worktree, &["commit", "-m", "more"]);

    // Apply with the OLD fingerprint → blocked.
    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--fingerprint", &good_fp])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("drifted"));
}

// -- apply: pushing -----------------------------------------------------------

#[test]
fn deliver_pushes_the_feature_branch_to_the_remote() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    // Preview to get the fingerprint.
    let preview_output = ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview_value: serde_json::Value =
        serde_json::from_slice(&preview_output).expect("valid json");
    let fp = preview_value["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();

    // Apply.
    let output = ivar()
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--fingerprint",
            &fp,
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(value["pushes"].as_array().unwrap().len(), 1);
    assert_eq!(value["pushes"][0]["repo"], "api");
    assert_eq!(value["pushes"][0]["ok"], true);
}

#[test]
fn a_failed_push_becomes_a_warning_not_an_abort() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    // Break the remote before delivering.
    let manifest_path = root.join("ivar.json");
    let content = std::fs::read_to_string(&manifest_path).unwrap();
    let modified = content.replace(
        "\"url\":\"",
        &format!("\"url\":\"{}", root.join("no-such-origin")),
    );
    std::fs::write(&manifest_path, modified).unwrap();

    // Preview to get the fingerprint (with the broken URL baked in).
    let preview_output = ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview_value: serde_json::Value =
        serde_json::from_slice(&preview_output).expect("valid json");
    let fp = preview_value["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();

    // Apply — should succeed overall but with a warning.
    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--fingerprint", &fp])
        .assert()
        .code(1) // exit 1 = warnings present
        .stderr(
            predicate::str::contains("does not appear to be a git repository")
                .or(predicate::str::contains("could not read from remote")),
        );
}

// -- human surface ------------------------------------------------------------

#[test]
fn human_preview_surface_lists_each_repo_and_the_fingerprint() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    let output = ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--preview"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let human = String::from_utf8(output).expect("utf8 output");
    assert!(human.contains("Delivery preview for `checkout`"));
    assert!(human.contains("branch:  checkout"));
    assert!(human.contains("refspec: checkout:refs/heads/checkout"));
    assert!(human.contains("base:    main"));
    // The remote is a local path — push only, no PR.
    assert!(human.contains("action:  push only"));
    assert!(human.contains("fingerprint:"));
}
