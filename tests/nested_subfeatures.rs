//! End-to-end journeys for nested subfeatures through the compiled binary:
//! creation/reparenting, leaves-first integration into immediate parents,
//! PR-protected merges through the fake `gh`, partial multi-repo resume, and
//! the deletion/lifecycle gates.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/integration.rs"]
mod common;

use camino::{Utf8Path, Utf8PathBuf};
use common::{FakeGh, hall_root, ivar, seeded_repo};
use predicates::prelude::predicate;

/// Run git in `cwd` with a fixed identity.
fn git(cwd: &Utf8Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(["-c", "user.name=ivar tests"])
        .args(["-c", "user.email=tests@ivar.invalid"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed in {cwd}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A hall with one repo (`api`), the child's SPDD plan approved, and the
/// parent/child tree created through the CLI. `checks` become the api repo's
/// ordered verification checks (via a hand-written v2 manifest, which also
/// exercises the migrate path when `migrate` is true).
fn nested_hall(checks: &[&str]) -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "main");
    // A v2 manifest with hall integration defaults and ordered checks.
    let checks_json: Vec<serde_json::Value> = checks
        .iter()
        .map(|check| serde_json::json!(check))
        .collect();
    let manifest = serde_json::json!({
        "version": 2,
        "name": "acme",
        "integration": { "via": "local", "strategy": "squash" },
        "providers": { "available": ["claude-code"], "default": "claude-code" },
        "repos": [
            { "name": "api", "url": origin.as_str(), "default_branch": "main", "checks": checks_json }
        ],
    });
    std::fs::write(
        root.join("ivar.json"),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();
    ivar().current_dir(&root).arg("sync").assert().success();

    ivar()
        .current_dir(&root)
        .args(["feature", "create", "parent"])
        .assert()
        .success();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "child", "--parent", "parent"])
        .assert()
        .success();

    // Promote api into both and put a commit on the child.
    for feature in ["parent", "child"] {
        ivar()
            .current_dir(&root)
            .args(["feature", "promote", feature, "api"])
            .assert()
            .success();
    }
    let child_wt = root.join(".ivar/repos/api/child");
    std::fs::write(child_wt.join("work.md"), "child work\n").unwrap();
    git(&child_wt, &["add", "work.md"]);
    git(&child_wt, &["commit", "-m", "child work"]);

    approve_plan(&root, "child");
    (guard, root)
}

/// Scaffold and approve the child's plan gate through the CLI.
fn approve_plan(root: &Utf8Path, feature: &str) {
    ivar()
        .current_dir(root)
        .args(["plan", "create", feature])
        .assert()
        .success();
    for gate in ["requirements", "analysis", "plan"] {
        ivar()
            .current_dir(root)
            .args(["plan", "approve", feature, gate])
            .assert()
            .success();
    }
}

fn json_output(root: &Utf8Path, args: &[&str]) -> serde_json::Value {
    // A successful run may exit 1 when it carries warnings (e.g. the
    // integration close notice); only a real failure (exit 2) is refused.
    let output = ivar()
        .current_dir(root)
        .args(args)
        .arg("--json")
        .assert()
        .code(predicate::in_iter([0, 1]))
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).expect("valid json")
}

fn failure_output(root: &Utf8Path, args: &[&str]) -> serde_json::Value {
    let output = ivar()
        .current_dir(root)
        .args(args)
        .arg("--json")
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).expect("valid json")
}

// -- creation, reparent, and the tree ---------------------------------------

#[test]
fn the_cli_can_create_reparent_and_refuse_after_work_starts() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "main");
    let manifest = serde_json::json!({
        "version": 2,
        "name": "acme",
        "integration": { "via": "local", "strategy": "squash" },
        "providers": { "available": ["claude-code"], "default": "claude-code" },
        "repos": [
            { "name": "api", "url": origin.as_str(), "default_branch": "main", "checks": ["true"] }
        ],
    });
    std::fs::write(
        root.join("ivar.json"),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();
    ivar().current_dir(&root).arg("sync").assert().success();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "parent"])
        .assert()
        .success();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "parent-b"])
        .assert()
        .success();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "child", "--parent", "parent"])
        .assert()
        .success();

    // The still-pristine child reparents: parent and derived base update
    // together in the one record.
    let reparented = json_output(
        &root,
        &["feature", "reparent", "child", "--parent", "parent-b"],
    );
    assert_eq!(reparented["new_parent"], "parent-b");
    let child_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join(".ivar/features/child/feature.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(child_json["parent"], "parent-b");
    assert_eq!(child_json["base"], "parent-b");

    // A leaf under the child makes further reparenting refuse.
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "leaf", "--parent", "child"])
        .assert()
        .success();
    let failure = failure_output(
        &root,
        &["feature", "reparent", "child", "--parent", "parent"],
    );
    assert_eq!(failure["code"], "feature.reparent_work_started");

    // Reparenting after a promotion refuses too. (The promote warns — the
    // reparented base branch does not exist in the repo yet — and still
    // succeeds with exit 1.)
    ivar()
        .current_dir(&root)
        .args(["feature", "promote", "child", "api"])
        .assert()
        .code(predicate::in_iter([0, 1]));
    let failure = failure_output(
        &root,
        &["feature", "reparent", "child", "--parent", "parent"],
    );
    assert_eq!(failure["code"], "feature.reparent_work_started");

    // List/status expose the tree with depth and immediate targets.
    let status = json_output(&root, &["feature", "status", "parent-b", "--recursive"]);
    let tree = status["tree"].as_array().unwrap();
    let rendered: Vec<(String, usize)> = tree
        .iter()
        .map(|entry| {
            (
                entry["feature"].as_str().unwrap().to_owned(),
                entry["depth"].as_u64().unwrap() as usize,
            )
        })
        .collect();
    assert_eq!(
        rendered,
        [
            ("parent-b".to_owned(), 0),
            ("child".to_owned(), 1),
            ("leaf".to_owned(), 2)
        ]
    );
}

// -- leaves-first local integration ------------------------------------------

#[test]
fn leaf_integrates_into_child_which_integrates_into_parent_for_all_strategies() {
    for strategy in ["squash", "merge", "rebase"] {
        let (_guard, root) = nested_hall(&["true"]);
        // Create a leaf under the child with its own commit.
        ivar()
            .current_dir(&root)
            .args(["feature", "create", "leaf", "--parent", "child"])
            .assert()
            .success();
        ivar()
            .current_dir(&root)
            .args(["feature", "promote", "leaf", "api"])
            .assert()
            .success();
        let leaf_wt = root.join(".ivar/repos/api/leaf");
        std::fs::write(leaf_wt.join("leaf.md"), "leaf work\n").unwrap();
        git(&leaf_wt, &["add", "leaf.md"]);
        git(&leaf_wt, &["commit", "-m", "leaf work"]);
        approve_plan(&root, "leaf");

        // Leaves first: the child refuses while the leaf blocks it.
        let failure = failure_output(&root, &["feature", "integrate", "child"]);
        assert_eq!(failure["code"], "feature.descendants_block");

        // The leaf integrates into the child.
        let leaf_run = json_output(
            &root,
            &["feature", "integrate", "leaf", "--strategy", strategy],
        );
        assert_eq!(leaf_run["closed_integrated"], true);
        assert_eq!(leaf_run["parent"], "child");
        assert_eq!(leaf_run["repos"][0]["status"], "integrated");
        // The child worktree now carries the leaf's work.
        let child_wt = root.join(".ivar/repos/api/child");
        assert!(child_wt.join("leaf.md").is_file());

        // Then the child integrates into the parent.
        let child_run = json_output(
            &root,
            &["feature", "integrate", "child", "--strategy", strategy],
        );
        assert_eq!(child_run["closed_integrated"], true);
        assert_eq!(child_run["parent"], "parent");
        let parent_wt = root.join(".ivar/repos/api/parent");
        assert!(parent_wt.join("leaf.md").is_file());
        assert!(parent_wt.join("work.md").is_file());

        // Receipts carry source/target/result evidence.
        let child_json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(".ivar/features/child/feature.json")).unwrap(),
        )
        .unwrap();
        let receipt = &child_json["promotions"]["api"]["integration_receipt"];
        assert_eq!(receipt["target_branch"], "parent");
        assert!(!receipt["source_sha"].as_str().unwrap().is_empty());
        assert!(!receipt["result_sha"].as_str().unwrap().is_empty());
        assert_eq!(
            receipt["verification"]["command_fingerprint"]
                .as_str()
                .unwrap()
                .len(),
            64
        );

        // Refs are retained: the leaf's branch and worktree survive.
        let leaf_ref = std::process::Command::new("git")
            .args([
                "--git-dir",
                root.join(".ivar/repos/api/.bare").as_str(),
                "rev-parse",
                "--verify",
                "refs/heads/leaf",
            ])
            .output()
            .unwrap();
        assert!(
            leaf_ref.status.success(),
            "the leaf branch must be retained"
        );
        assert!(root.join(".ivar/repos/api/leaf").is_dir());

        // The root remains deliverable, not integrated.
        let failure = failure_output(&root, &["feature", "integrate", "parent"]);
        assert_eq!(failure["code"], "integration.root_refused");
    }
}

// -- PR-protected merges through the fake gh ---------------------------------

/// A hall whose repo is promoted into parent and child, with the fake `gh`
/// on PATH. The PR half is driven entirely by the fake — the manifest points
/// at the local origin, and `--via pr` routes through `gh`.
fn github_hall(checks: &[&str]) -> (tempfile::TempDir, Utf8PathBuf, FakeGh) {
    let (guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "main");
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("ivar.json")).unwrap()).unwrap();
    let checks_json: Vec<serde_json::Value> = checks
        .iter()
        .map(|check| serde_json::json!(check))
        .collect();
    value["repos"] = serde_json::json!([
        {
            "name": "api",
            "url": origin.as_str(),
            "default_branch": "main",
            "checks": checks_json,
        }
    ]);
    std::fs::write(
        root.join("ivar.json"),
        serde_json::to_string(&value).unwrap(),
    )
    .unwrap();
    ivar().current_dir(&root).arg("sync").assert().success();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "parent"])
        .assert()
        .success();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "child", "--parent", "parent"])
        .assert()
        .success();
    for feature in ["parent", "child"] {
        ivar()
            .current_dir(&root)
            .args(["feature", "promote", feature, "api"])
            .assert()
            .success();
    }
    let child_wt = root.join(".ivar/repos/api/child");
    std::fs::write(child_wt.join("work.md"), "child work\n").unwrap();
    git(&child_wt, &["add", "work.md"]);
    git(&child_wt, &["commit", "-m", "child work"]);
    approve_plan(&root, "child");

    let fake = FakeGh::install(&root);
    (guard, root, fake)
}

/// `ivar`, with the fake `gh` first on PATH.
fn ivar_on_github(root: &Utf8Path, fake: &FakeGh) -> assert_cmd::Command {
    let mut command = ivar();
    let path = std::env::var("PATH").unwrap_or_default();
    command
        .env("PATH", format!("{}:{path}", fake.dir))
        .env("GH_FAKE_STATE", fake.state.as_str())
        .env("GH_FAKE_LOG", fake.log.as_str())
        .env("GH_FAKE_CHECKS", fake.checks.as_str());
    command.current_dir(root);
    command
}

#[test]
fn a_pr_integration_checks_merges_and_observes_through_the_fake_gh() {
    let (_guard, root, fake) = github_hall(&["true"]);

    // Integrate via PR: create, check, merge, observe. (The close notice is a
    // warning, so the run exits 1 — json_output tolerates that.)
    let value = json_gh_output(
        &root,
        &fake,
        &[
            "feature",
            "integrate",
            "child",
            "--via",
            "pr",
            "--strategy",
            "squash",
        ],
    );
    assert_eq!(value["closed_integrated"], true);
    assert_eq!(value["closed_integrated"], true);
    assert_eq!(value["policy"]["via"], "pr");
    let repo = &value["repos"][0];
    assert_eq!(repo["status"], "integrated");
    assert!(repo["pr_url"].as_str().unwrap().contains("/pull/"));

    // The PR was created, checked, merged, and observed.
    let log = fake.log();
    assert!(log.contains("pr create"), "{log}");
    assert!(log.contains("pr checks"), "{log}");
    assert!(log.contains("pr merge"), "{log}");
    assert!(log.contains("pr view"), "{log}");

    // The parent's local worktree was fetched forward to the merged result.
    let parent_wt = root.join(".ivar/repos/api/parent");
    assert!(parent_wt.join("work.md").is_file());

    // A rerun reuses the fresh receipt.
    let value = json_gh_output(
        &root,
        &fake,
        &["feature", "integrate", "child", "--via", "pr"],
    );
    assert_eq!(value["repos"][0]["status"], "reused");
    assert_eq!(value["closed_integrated"], false, "a rerun never reopens");
}

#[test]
fn a_head_movement_after_the_pr_blocks_the_merge_with_match_head_commit() {
    let (_guard, root, fake) = github_hall(&["true"]);

    // First run: a pending required check creates the PR but stops before
    // the merge — status `pending`, no receipt.
    let url = "https://github.com/acme/pull/1";
    fake.set_check(url, "ci", "pending", "pending");
    let first = json_gh_output(
        &root,
        &fake,
        &[
            "feature",
            "integrate",
            "child",
            "--via",
            "pr",
            "--strategy",
            "squash",
        ],
    );
    assert_eq!(first["repos"][0]["status"], "pending");
    assert_eq!(first["closed_integrated"], false);

    // The child head moves after the PR was opened: a new commit lands.
    let child_wt = root.join(".ivar/repos/api/child");
    std::fs::write(child_wt.join("more.md"), "more\n").unwrap();
    git(&child_wt, &["add", "more.md"]);
    git(&child_wt, &["commit", "-m", "more"]);
    // And the checks pass now.
    fake.set_check(url, "ci", "pass", "completed");

    // Rerun: the PR's recorded head no longer matches the source, so the
    // merge is refused via `--match-head-commit` — exactly like the real gh.
    // The refused repo is reported as failed (exit 1, with the warning), and
    // no receipt is recorded.
    let rerun = json_gh_output(
        &root,
        &fake,
        &[
            "feature",
            "integrate",
            "child",
            "--via",
            "pr",
            "--strategy",
            "squash",
        ],
    );
    assert_eq!(rerun["repos"][0]["status"], "failed");
    assert!(
        rerun["repos"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("does not match head"),
        "was: {rerun}"
    );
    assert_eq!(rerun["closed_integrated"], false);
}

/// `ivar` on the fake gh, accepting exit 0 or 1 (the close warning).
fn json_gh_output(root: &Utf8Path, fake: &FakeGh, args: &[&str]) -> serde_json::Value {
    let output = ivar_on_github(root, fake)
        .args(args)
        .arg("--json")
        .assert()
        .code(predicate::in_iter([0, 1]))
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).expect("valid json")
}

// -- partial multi-repo resume ------------------------------------------------

/// A hall with api (checks pass) and web (checks fail), child promoting both,
/// the plan approved.
fn partial_hall() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let origins = root.parent().unwrap().join("origins");
    let api_origin = seeded_repo(&origins.join("api"), "main");
    let web_origin = seeded_repo(&origins.join("web"), "main");
    let manifest = serde_json::json!({
        "version": 2,
        "name": "acme",
        "integration": { "via": "local", "strategy": "squash" },
        "providers": { "available": ["claude-code"], "default": "claude-code" },
        "repos": [
            { "name": "api", "url": api_origin.as_str(), "default_branch": "main", "checks": ["true"] },
            { "name": "web", "url": web_origin.as_str(), "default_branch": "main", "checks": ["exit 1"] },
        ],
    });
    std::fs::write(
        root.join("ivar.json"),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();
    ivar().current_dir(&root).arg("sync").assert().success();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "parent"])
        .assert()
        .success();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "child", "--parent", "parent"])
        .assert()
        .success();
    for feature in ["parent", "child"] {
        for repo in ["api", "web"] {
            ivar()
                .current_dir(&root)
                .args(["feature", "promote", feature, repo])
                .assert()
                .success();
        }
    }
    for repo in ["api", "web"] {
        let wt = root.join(format!(".ivar/repos/{repo}/child"));
        std::fs::write(wt.join("work.md"), "work\n").unwrap();
        git(&wt, &["add", "work.md"]);
        git(&wt, &["commit", "-m", "child work"]);
    }
    approve_plan(&root, "child");
    (guard, root)
}

#[test]
fn a_partial_integration_is_resumable_and_never_atomic() {
    let (_guard, root) = partial_hall();

    // First run: api integrates, web's checks fail — the child does NOT close.
    let run = json_output(&root, &["feature", "integrate", "child"]);
    assert_eq!(run["closed_integrated"], false);
    assert_eq!(run["repos"][0]["status"], "integrated");
    assert_eq!(run["repos"][1]["status"], "failed");

    // api's receipt exists and is persisted; web's does not.
    let child_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join(".ivar/features/child/feature.json")).unwrap(),
    )
    .unwrap();
    assert!(child_json["promotions"]["api"]["integration_receipt"].is_object());
    assert!(child_json["promotions"]["web"]["integration_receipt"].is_null());

    // api is now individually immutable: demote refuses before any mutation.
    let failure = failure_output(&root, &["feature", "demote", "child", "api"]);
    assert_eq!(failure["code"], "feature.promotion_integration_immutable");

    // Repair web's checks, then rerun: api is reused, web integrates, and the
    // child closes integrated.
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("ivar.json")).unwrap()).unwrap();
    for repo in manifest["repos"].as_array_mut().unwrap() {
        if repo["name"] == "web" {
            repo["checks"] = serde_json::json!(["true"]);
        }
    }
    std::fs::write(
        root.join("ivar.json"),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();

    let api_parent_before = std::process::Command::new("git")
        .args([
            "--git-dir",
            root.join(".ivar/repos/api/.bare").as_str(),
            "rev-parse",
            "parent",
        ])
        .output()
        .unwrap();
    let api_parent_before = String::from_utf8_lossy(&api_parent_before.stdout)
        .trim()
        .to_owned();

    let rerun = json_output(&root, &["feature", "integrate", "child"]);
    assert_eq!(rerun["closed_integrated"], true);
    assert_eq!(rerun["repos"][0]["status"], "reused");
    assert_eq!(rerun["repos"][1]["status"], "integrated");

    // api's parent branch is byte-for-byte unchanged by the rerun.
    let api_parent_after = std::process::Command::new("git")
        .args([
            "--git-dir",
            root.join(".ivar/repos/api/.bare").as_str(),
            "rev-parse",
            "parent",
        ])
        .output()
        .unwrap();
    let api_parent_after = String::from_utf8_lossy(&api_parent_after.stdout)
        .trim()
        .to_owned();
    assert_eq!(api_parent_before, api_parent_after);
}

// -- parent-promotion mode matrix ---------------------------------------------

#[test]
fn a_missing_parent_promotion_blocks_noninteractive_runs_with_the_exact_command() {
    let (_guard, root) = partial_hall();
    // The parent promotes both repos in partial_hall; demote web from the
    // parent so the child promotes it but the parent does not.
    ivar()
        .current_dir(&root)
        .args(["feature", "demote", "parent", "web"])
        .assert()
        .success();

    let failure = failure_output(&root, &["feature", "integrate", "child"]);
    assert_eq!(failure["code"], "integration.parent_promotion_required");
    let fix = failure["fix_actions"][0]["command"].as_str().unwrap();
    assert_eq!(fix, "ivar feature promote parent web");
}

// -- deletion and lifecycle gates ---------------------------------------------

#[test]
fn parent_deletion_is_blocked_until_every_descendant_is_deleted() {
    let (_guard, root) = nested_hall(&["true"]);
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "leaf", "--parent", "child"])
        .assert()
        .success();

    let failure = failure_output(&root, &["feature", "delete", "parent"]);
    assert_eq!(failure["code"], "feature.has_descendants");

    // Leaves first, then the parent goes.
    ivar()
        .current_dir(&root)
        .args(["feature", "delete", "leaf"])
        .assert()
        .success();
    let failure = failure_output(&root, &["feature", "delete", "parent"]);
    assert_eq!(failure["code"], "feature.has_descendants");
    ivar()
        .current_dir(&root)
        .args(["feature", "delete", "child"])
        .assert()
        .success();
    ivar()
        .current_dir(&root)
        .args(["feature", "delete", "parent"])
        .assert()
        .success();
    assert!(!root.join(".ivar/features/parent").exists());
}

#[test]
fn a_child_cannot_deliver_and_an_integrated_child_cannot_reopen() {
    let (_guard, root) = nested_hall(&["true"]);

    // A child refuses delivery with the integrate command.
    let failure = failure_output(&root, &["feature", "deliver", "child", "--preview"]);
    assert_eq!(failure["code"], "deliver.child_requires_integration");

    // Integrate the child, then verify the integrated outcome is final: close
    // cannot replace it, and the child cannot deliver or be reparented.
    let run = json_output(&root, &["feature", "integrate", "child"]);
    assert_eq!(run["closed_integrated"], true);

    let reopened = json_output(
        &root,
        &["feature", "close", "child", "--outcome", "delivered"],
    );
    assert_eq!(reopened["already_closed"], true);
    assert_eq!(reopened["outcome"], "integrated");

    let failure = failure_output(&root, &["feature", "deliver", "child", "--preview"]);
    assert_eq!(failure["code"], "deliver.child_requires_integration");
}
