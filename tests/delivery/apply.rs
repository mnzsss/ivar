use crate::common::{FakeGh, git, hall_root, ivar, seeded_repo};
use crate::support::{
    approve_through_plan, as_github_remotes, deliver_on_github_expecting_warnings,
    preview_fingerprint, setup_two_repo_hall,
};

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

#[test]
fn a_repo_whose_push_failed_gets_no_pull_request() {
    use std::os::unix::fs::PermissionsExt;

    let (_guard, root) = hall_root();
    setup_two_repo_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);
    let hook = root
        .parent()
        .unwrap()
        .join("origins/web/.git/hooks/pre-receive");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let applied = deliver_on_github_expecting_warnings(&root, &fake, &rewrites, "checkout");

    let pushes = applied["pushes"].as_array().expect("pushes array");
    let web = pushes
        .iter()
        .find(|push| push["repo"] == "web")
        .expect("web push result");
    assert_eq!(web["ok"], false, "the rejected push is reported");
    assert!(web["pr"].is_null(), "no PR for an unpushed repo: {web}");
    assert_eq!(
        fake.log().matches("pr create").count(),
        1,
        "only the pushed repo gets a PR: {}",
        fake.log()
    );
}

#[test]
fn deliver_only_pushes_the_selected_repos_end_to_end() {
    let (_guard, root) = hall_root();
    setup_two_repo_hall(&root);
    approve_through_plan(&root, "checkout");
    let json = |output: Vec<u8>| -> serde_json::Value {
        serde_json::from_slice(&output).expect("valid json")
    };

    let preview = json(
        ivar()
            .current_dir(&root)
            .args([
                "feature",
                "deliver",
                "checkout",
                "--only",
                "api",
                "--preview",
                "--json",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    );
    let fingerprint = preview["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    let applied = json(
        ivar()
            .current_dir(&root)
            .args([
                "feature",
                "deliver",
                "checkout",
                "--only",
                "api",
                "--fingerprint",
                &fingerprint,
                "--json",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    );

    let pushed: Vec<&str> = applied["pushes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|push| push["repo"].as_str().unwrap())
        .collect();
    assert_eq!(pushed, ["api"]);
    let origins = root.parent().unwrap().join("origins");
    let carries_checkout = |repo: &str| {
        std::process::Command::new("git")
            .args(["-C", origins.join(repo).as_str()])
            .args(["rev-parse", "--verify", "-q", "refs/heads/checkout"])
            .status()
            .unwrap()
            .success()
    };
    assert!(carries_checkout("api"), "the selected repo is pushed");
    assert!(!carries_checkout("web"), "an unselected repo is left alone");
}
