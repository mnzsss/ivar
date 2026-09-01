use crate::common::{declare_repos, hall_root, ivar, seeded_repo};
use crate::support::{approve_through_plan, preview_fingerprint, setup_deliver_hall};
use predicates::prelude::*;

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

#[test]
fn the_preview_reports_the_plan_gate_without_refusing() {
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
    assert_eq!(value["preview"]["plan_gate"], "pending");

    approve_through_plan(&root, "checkout");

    let output = ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(value["preview"]["plan_gate"], "approved");
}

#[test]
fn approving_the_plan_after_a_preview_drifts_the_fingerprint() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);

    let stale = preview_fingerprint(&root, "checkout");
    approve_through_plan(&root, "checkout");

    // The gate state is part of what the human approved, so crossing it
    // is drift like any other — the preview has to be taken again.
    ivar()
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--fingerprint", &stale])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("drifted"));
}
