use crate::common::{FakeGh, git, hall_root, ivar, seeded_repo};
use crate::support::{
    approve_through_plan, as_github_remotes, as_unreachable_github_remote,
    deliver_on_github_expecting_warnings, preview_fingerprint, setup_deliver_hall,
    setup_deliver_hall_with_base,
};
use predicates::prelude::*;

#[test]
fn apply_requires_a_preview_fingerprint() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");

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
    approve_through_plan(&root, "checkout");

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

    // Apply with the OLD fingerprint -> blocked.
    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--fingerprint", &good_fp])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("drifted"));
}

// -- apply: the plan gate -----------------------------------------------------

#[test]
fn apply_is_refused_while_the_plan_gate_is_not_approved() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("plan"))
        .stderr(predicate::str::contains("ivar plan approve checkout plan"));
}

#[test]
fn apply_is_refused_even_with_a_matching_fingerprint_while_the_gate_is_pending() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    // A fingerprint straight off the preview — the drift gate is satisfied, and
    // the plan gate still refuses. The two are independent.
    let fp = preview_fingerprint(&root, "checkout");

    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--fingerprint", &fp])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("ivar plan approve checkout plan"));
}

#[test]
fn approving_only_the_upstream_gates_is_not_enough() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    ivar()
        .current_dir(&root)
        .args(["plan", "create", "checkout"])
        .assert()
        .success();
    for gate in ["requirements", "analysis"] {
        ivar()
            .current_dir(&root)
            .args(["plan", "approve", "checkout", gate])
            .assert()
            .success();
    }

    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("ivar plan approve checkout plan"));
}

#[test]
fn invalidating_the_plan_gate_closes_delivery_again() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");

    ivar()
        .current_dir(&root)
        .args(["plan", "invalidate", "checkout", "plan"])
        .assert()
        .success();

    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("ivar plan approve checkout plan"));
}

// -- apply: pushing -----------------------------------------------------------

#[test]
fn deliver_pushes_the_feature_branch_to_the_remote() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");

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
    approve_through_plan(&root, "checkout");

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

// -- apply: the base verdict ---------------------------------------------------
//
// `feature deliver` refuses to open or update a PR against a base that no
// longer supports it — merged and deleted, never delivered, moved on without
// a rebase, or simply unreachable. Each refusal is per repo (the push still
// lands) and never touches the network beyond what `remote_branch_tip`
// already reaches for; see `domain::feature::delivery` for the pure
// classification these tests exercise end to end.

#[test]
fn delivering_with_an_unreachable_remote_reports_the_base_as_unconfirmed_never_absent() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_unreachable_github_remote(&root);

    let applied = deliver_on_github_expecting_warnings(&root, &fake, &rewrites, "checkout");

    let warnings = applied["warnings"].as_array().expect("warnings array");
    let warning = warnings
        .iter()
        .find(|w| w["code"] == "feature.base_unconfirmed")
        .unwrap_or_else(|| panic!("no base_unconfirmed warning; warnings were: {warnings:?}"));
    let what = warning["what"].as_str().unwrap().to_lowercase();
    assert!(
        !what.contains("absent"),
        "an unanswered remote must never be reported as an absent base: {what}"
    );
    assert!(applied["preview"]["repos"][0]["pr_url"].is_null());
    assert_eq!(
        fake.log().matches("pr create").count(),
        0,
        "no PR may be attempted against an unconfirmed base"
    );
}

#[test]
fn delivering_with_a_base_that_moved_refuses_the_pr_but_still_pushes() {
    let (_guard, root) = hall_root();
    setup_deliver_hall_with_base(&root, false);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Advance `develop` past what `checkout` was cut from, and pull that
    // straight into the bare clone's own `develop` ref — simulating that
    // ivar's local knowledge of the base has moved on, since `ivar sync`
    // itself only ever keeps the default branch's worktree current.
    let origin = root.parent().unwrap().join("origins/api");
    git(&origin, &["checkout", "develop"]);
    std::fs::write(origin.join("develop-later.txt"), "later\n").unwrap();
    git(&origin, &["add", "develop-later.txt"]);
    git(&origin, &["commit", "-m", "later develop work"]);
    git(&origin, &["checkout", "main"]);
    let bare = root.join(".ivar/repos/api/.bare");
    git(&bare, &["fetch", origin.as_str(), "develop:develop"]);

    let applied = deliver_on_github_expecting_warnings(&root, &fake, &rewrites, "checkout");

    assert_eq!(
        applied["pushes"][0]["ok"], true,
        "pushing raw commits does not depend on the base"
    );
    let warnings = applied["warnings"].as_array().expect("warnings array");
    assert!(
        warnings.iter().any(|w| w["code"] == "feature.base_moved"),
        "warnings were: {warnings:?}"
    );
    assert!(applied["preview"]["repos"][0]["pr_url"].is_null());
    assert_eq!(fake.log().matches("pr create").count(), 0);
}

/// The bare clone's own `develop` ref is never re-fetched by anything this
/// test runs — `ivar sync` only ever keeps the default branch's worktree
/// current — so this is the ordinary case: the remote has moved on and
/// nothing local knows it yet. The check must ask the remote's own tip, not
/// trust a local ref that still (trivially) looks like an ancestor.
#[test]
fn delivering_with_a_base_that_moved_only_on_the_remote_still_refuses_the_pr() {
    let (_guard, root) = hall_root();
    setup_deliver_hall_with_base(&root, false);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Advance `develop` on the remote only — no fetch follows, so the bare
    // clone's local `develop` ref stays exactly where it was at promote
    // time, still (trivially) an ancestor of `checkout`.
    let origin = root.parent().unwrap().join("origins/api");
    git(&origin, &["checkout", "develop"]);
    std::fs::write(origin.join("develop-later.txt"), "later\n").unwrap();
    git(&origin, &["add", "develop-later.txt"]);
    git(&origin, &["commit", "-m", "later develop work"]);
    git(&origin, &["checkout", "main"]);

    let applied = deliver_on_github_expecting_warnings(&root, &fake, &rewrites, "checkout");

    assert_eq!(applied["pushes"][0]["ok"], true);
    let warnings = applied["warnings"].as_array().expect("warnings array");
    assert!(
        warnings.iter().any(|w| w["code"] == "feature.base_moved"),
        "a base advanced only on the remote must still refuse — warnings were: {warnings:?}"
    );
    assert!(applied["preview"]["repos"][0]["pr_url"].is_null());
    assert_eq!(fake.log().matches("pr create").count(), 0);
}

#[test]
fn delivering_with_a_merged_and_deleted_base_refuses_the_pr_with_a_rebase_onto_default_fix() {
    let (_guard, root) = hall_root();
    let origin = setup_deliver_hall_with_base(&root, true);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // The base shipped and its branch was deleted — GitHub's usual
    // auto-delete-on-merge. ivar's bare clone keeps its own, now stale,
    // local `develop` ref, exactly as a developer's clone would.
    git(&origin, &["branch", "-D", "develop"]);

    let applied = deliver_on_github_expecting_warnings(&root, &fake, &rewrites, "checkout");

    assert_eq!(applied["pushes"][0]["ok"], true);
    let warnings = applied["warnings"].as_array().expect("warnings array");
    let warning = warnings
        .iter()
        .find(|w| w["code"] == "feature.base_merged_and_deleted")
        .unwrap_or_else(|| panic!("warnings were: {warnings:?}"));
    assert!(warning["what"].as_str().unwrap().contains("develop"));
    assert!(applied["preview"]["repos"][0]["pr_url"].is_null());
}

#[test]
fn delivering_with_a_never_delivered_base_refuses_the_pr_with_a_deliver_parent_first_fix() {
    let (_guard, root) = hall_root();
    let origin = setup_deliver_hall_with_base(&root, false);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // The base's branch is gone, and — unlike the merged-and-deleted case —
    // it was never merged into `main` first: nothing confirms it shipped.
    git(&origin, &["branch", "-D", "develop"]);

    let applied = deliver_on_github_expecting_warnings(&root, &fake, &rewrites, "checkout");

    assert_eq!(applied["pushes"][0]["ok"], true);
    let warnings = applied["warnings"].as_array().expect("warnings array");
    assert!(
        warnings
            .iter()
            .any(|w| w["code"] == "feature.base_never_delivered"),
        "warnings were: {warnings:?}"
    );
    assert!(applied["preview"]["repos"][0]["pr_url"].is_null());
}

// -- end to end ---------------------------------------------------------------

/// Zero to delivered, through CLI verbs only.
///
/// The point of this test is what it does *not* do: no file under `.ivar/` is
/// written by hand at any step. Every state the run passes through — the hall,
/// the repo, the feature, the promotion, the four planning artifacts, the three
/// crossed gates, the preview fingerprint — is reachable by running `ivar`.
/// A gate that could only be crossed by editing JSON would fail here.
#[test]
fn the_whole_path_from_an_empty_directory_to_a_pushed_branch_runs_on_cli_verbs_only() {
    let (_guard, root) = hall_root();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "main");

    ivar().current_dir(&root).arg("init").assert().success();
    ivar()
        .current_dir(&root)
        .args(["repo", "add", "api", origin.as_str()])
        .assert()
        .success();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "checkout"])
        .assert()
        .success();
    ivar()
        .current_dir(&root)
        .args(["feature", "promote", "checkout", "api"])
        .assert()
        .success();

    // Something to deliver.
    let worktree = root.join(".ivar/repos/api/checkout");
    std::fs::write(worktree.join("work.md"), "work\n").unwrap();
    git(&worktree, &["add", "work.md"]);
    git(&worktree, &["commit", "-m", "work"]);

    approve_through_plan(&root, "checkout");

    let fingerprint = preview_fingerprint(&root, "checkout");
    ivar()
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--fingerprint",
            &fingerprint,
        ])
        .assert()
        .success();

    // The branch actually landed on the origin.
    let output = std::process::Command::new("git")
        .args(["-C", origin.as_str(), "rev-parse", "--verify"])
        .arg("refs/heads/checkout")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "the feature branch never reached the origin: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
