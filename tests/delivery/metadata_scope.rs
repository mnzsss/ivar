//! Pull-request metadata scope and inheritance.

use crate::common::{FakeGh, declare_repos, git, hall_root, ivar, seeded_repo};
use crate::support::{approve_through_plan, as_github_remotes, ivar_on_github, setup_deliver_hall};

/// Global custom creation: `--name` and `--body` as global flags create a PR
/// with those values as title and body.
#[test]
fn global_custom_creation() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Preview with global title and body to get fingerprint
    let output = ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--name",
            "feat: global title",
            "--body",
            "global body inline",
            "--preview",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    let preview = &value["preview"];
    assert_eq!(preview["repos"][0]["action"], "new_pr");
    assert_eq!(
        preview["repos"][0]["pr_title"], "feat: global title",
        "global --name should set the PR title"
    );
    assert_eq!(
        preview["repos"][0]["pr_body"], "global body inline",
        "global --body should set the PR body"
    );

    // Apply with fingerprint - should create PR with the custom title/body
    let fp = preview["fingerprint"].as_str().unwrap().to_owned();
    let output2 = ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--fingerprint",
            &fp,
            "--name",
            "feat: global title",
            "--body",
            "global body inline",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value2: serde_json::Value = serde_json::from_slice(&output2).expect("valid json");
    assert_eq!(
        value2["preview"]["repos"][0]["pr_url"], "https://github.com/acme/pull/1",
        "PR should be created"
    );
    let log = fake.log();
    assert!(
        log.contains("pr create --base main --head checkout --title"),
        "gh pr create should be called with title flag"
    );
    assert!(
        log.contains("--title feat: global title"),
        "gh pr create should use the custom title: {log}"
    );
    assert!(
        log.contains("--body global body inline"),
        "gh pr create should use the custom body: {log}"
    );
}

/// Two repos with different metadata values: each `--repo` group supplies
/// independent title/body for that repository.
#[test]
fn two_repos_with_different_values() {
    let (_guard, root) = hall_root();
    let origins = root.parent().unwrap().join("origins");
    let api = seeded_repo(&origins.join("api"), "main");
    let web = seeded_repo(&origins.join("web"), "main");
    declare_repos(&root, &[("api", &api, "main"), ("web", &web, "main")]);
    ivar().current_dir(&root).arg("sync").assert().success();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "checkout"])
        .assert()
        .success();

    for repo in ["api", "web"] {
        ivar()
            .current_dir(&root)
            .args(["feature", "promote", "checkout", repo])
            .assert()
            .success();
    }

    let worktree = root.join(".ivar/repos/api/checkout");
    std::fs::write(worktree.join("work.md"), "work\n").unwrap();
    git(&worktree, &["add", "work.md"]);
    git(&worktree, &["commit", "-m", "work"]);

    let worktree = root.join(".ivar/repos/web/checkout");
    std::fs::write(worktree.join("work.md"), "work\n").unwrap();
    git(&worktree, &["add", "work.md"]);
    git(&worktree, &["commit", "-m", "work"]);

    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Set global title/body, but api gets custom title/body via --repo
    let output = ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--name",
            "feat: global title",
            "--body",
            "global body",
            "--repo",
            "api",
            "--name",
            "feat(api): custom title",
            "--body",
            "custom api body",
            "--preview",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    let preview = &value["preview"];
    let repos = preview["repos"].as_array().expect("repos is an array");

    // api should have custom title and body from --repo
    let api_repo = &repos
        .iter()
        .find(|r| r["repo"] == "api")
        .expect("api repo found");
    assert_eq!(
        api_repo["pr_title"], "feat(api): custom title",
        "api should have custom title from --repo"
    );
    assert_eq!(
        api_repo["pr_body"], "custom api body",
        "api should have custom body from --repo"
    );

    // web should inherit global title and body
    let web_repo = &repos
        .iter()
        .find(|r| r["repo"] == "web")
        .expect("web repo found");
    assert_eq!(
        web_repo["pr_title"], "feat: global title",
        "web should inherit global title"
    );
    assert_eq!(
        web_repo["pr_body"], "global body",
        "web should inherit global body"
    );
}

/// Partial inheritance: repo override supplies only title, inheriting global body.
#[test]
fn partial_inheritance() {
    let (_guard, root) = hall_root();
    let origins = root.parent().unwrap().join("origins");
    let api = seeded_repo(&origins.join("api"), "main");
    let web = seeded_repo(&origins.join("web"), "main");
    declare_repos(&root, &[("api", &api, "main"), ("web", &web, "main")]);
    ivar().current_dir(&root).arg("sync").assert().success();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "checkout"])
        .assert()
        .success();

    for repo in ["api", "web"] {
        ivar()
            .current_dir(&root)
            .args(["feature", "promote", "checkout", repo])
            .assert()
            .success();
    }

    let worktree = root.join(".ivar/repos/api/checkout");
    std::fs::write(worktree.join("work.md"), "work\n").unwrap();
    git(&worktree, &["add", "work.md"]);
    git(&worktree, &["commit", "-m", "work"]);

    let worktree = root.join(".ivar/repos/web/checkout");
    std::fs::write(worktree.join("work.md"), "work\n").unwrap();
    git(&worktree, &["add", "work.md"]);
    git(&worktree, &["commit", "-m", "work"]);

    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Global title only; api gets custom title, inherits global body
    let output = ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--name",
            "feat: global title",
            "--body",
            "global body",
            "--repo",
            "api",
            "--name",
            "feat(api): custom title",
            "--preview",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    let preview = &value["preview"];
    let repos = preview["repos"].as_array().expect("repos is an array");

    let api_repo = &repos
        .iter()
        .find(|r| r["repo"] == "api")
        .expect("api repo found");
    assert_eq!(
        api_repo["pr_title"], "feat(api): custom title",
        "api title should come from --repo override"
    );
    assert_eq!(
        api_repo["pr_body"], "global body",
        "api body should be inherited from global --body"
    );

    let web_repo = &repos
        .iter()
        .find(|r| r["repo"] == "web")
        .expect("web repo found");
    assert_eq!(
        web_repo["pr_title"], "feat: global title",
        "web title should be inherited from global --name"
    );
    assert_eq!(
        web_repo["pr_body"], "global body",
        "web body should be inherited from global --body"
    );

    // Now test partial: global title only, api override supplies body only
    let output2 = ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--name",
            "feat: global title",
            "--repo",
            "web",
            "--body",
            "custom web body",
            "--preview",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value2: serde_json::Value = serde_json::from_slice(&output2).expect("valid json");
    let preview2 = &value2["preview"];
    let repos2 = preview2["repos"].as_array().expect("repos is an array");

    let api_repo2 = &repos2
        .iter()
        .find(|r| r["repo"] == "api")
        .expect("api repo found");
    assert_eq!(
        api_repo2["pr_title"], "feat: global title",
        "api title should be inherited from global --name"
    );
    assert!(
        api_repo2["pr_body"].is_null(),
        "api body should be absent (no global body, no api override)"
    );

    let web_repo2 = &repos2
        .iter()
        .find(|r| r["repo"] == "web")
        .expect("web repo found");
    assert_eq!(
        web_repo2["pr_title"], "feat: global title",
        "web title should be inherited from global --name"
    );
    assert_eq!(
        web_repo2["pr_body"], "custom web body",
        "web body should come from --repo override"
    );
}
