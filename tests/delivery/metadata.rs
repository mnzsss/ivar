use crate::common::{FakeGh, declare_repos, git, hall_root, ivar, seeded_repo};
use crate::support::{
    approve_through_plan, as_github_remotes, deliver_on_github, deliver_on_github_with,
    ivar_on_github, preview_on_github, preview_on_github_with, setup_deliver_hall,
};
use predicates::prelude::*;

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

/// Inline and cwd-relative md/txt body: `./body.md` and `body.txt` are resolved
/// correctly.
#[test]
fn inline_and_file_bodies() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");

    // Write body file
    let body_md = root.join("body.md");
    std::fs::write(&body_md, "Content from file\n").unwrap();
    let body_txt = root.join("body.txt");
    std::fs::write(&body_txt, "Content from txt\n").unwrap();

    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Inline body (just text, no ./ prefix or .md/.txt extension)
    let output = ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--name",
            "feat",
            "--body",
            "inline body text",
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
    let repo = &preview["repos"][0];
    assert_eq!(repo["pr_title"], "feat", "title should be set");
    assert_eq!(
        repo["pr_body"], "inline body text",
        "inline body should be stored"
    );

    // File body with ./ prefix for .md
    let output2 = ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--name",
            "feat",
            "--body",
            "./body.md",
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
    let repo2 = &preview2["repos"][0];
    assert_eq!(
        repo2["pr_body"], "Content from file\n",
        "file body ./body.md should resolve to file content"
    );

    // File body with ./ prefix for .txt
    let output3 = ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--name",
            "feat",
            "--body",
            "./body.txt",
            "--preview",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value3: serde_json::Value = serde_json::from_slice(&output3).expect("valid json");
    let preview3 = &value3["preview"];
    let repo3 = &preview3["repos"][0];
    assert_eq!(
        repo3["pr_body"], "Content from txt\n",
        "file body ./body.txt should resolve to file content"
    );

    // Non-prefixed body.txt is treated as inline text, not a file
    let output4 = ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--name",
            "feat",
            "--body",
            "body.md",
            "--preview",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value4: serde_json::Value = serde_json::from_slice(&output4).expect("valid json");
    let preview4 = &value4["preview"];
    let repo4 = &preview4["repos"][0];
    assert_eq!(
        repo4["pr_body"], "body.md",
        "non-prefixed body.md should be treated as inline text"
    );
}

/// Duplicate and unpromoted repo errors are rejected at preview.
#[test]
fn duplicate_and_unpromoted_repo_errors() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");

    // Duplicate --repo group for the same repo
    ivar()
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--name",
            "feat",
            "--repo",
            "api",
            "--repo",
            "api",
            "--preview",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("specified more than once"));

    // Unpromoted repo
    ivar()
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--name",
            "feat",
            "--repo",
            "unpromoted",
            "--preview",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("unpromoted"));
}

/// Land conflict: metadata cannot be used with --land.
#[test]
fn land_conflict() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");

    ivar()
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--land",
            "--name",
            "feat",
            "--preview",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("cannot be used in land mode"));
}

/// Metadata plus land rejection: passing metadata flags with --land is rejected.
#[test]
fn metadata_plus_land_rejection() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Land mode with metadata should be rejected at preview
    ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--land",
            "--name",
            "feat: should fail",
            "--preview",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("cannot be used in land mode"));

    // Land mode with --body should also be rejected
    ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--land",
            "--body",
            "./body.txt",
            "--preview",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("cannot be used in land mode"));
}

/// Existing PR title-only/body-only/both/no-op edits.
#[test]
fn existing_pr_edits() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Create initial PR with custom title and body
    let _preview = preview_on_github_with(
        &root,
        &fake,
        &rewrites,
        "checkout",
        &["--name", "feat: custom title", "--body", "custom body text"],
    );
    let applied = deliver_on_github_with(
        &root,
        &fake,
        &rewrites,
        "checkout",
        &["--name", "feat: custom title", "--body", "custom body text"],
    );
    assert_eq!(
        applied["preview"]["repos"][0]["pr_url"], "https://github.com/acme/pull/1",
        "PR should be created with custom metadata"
    );

    let create_log = fake.log();
    assert!(
        create_log.contains("--title"),
        "first create should use title flag: {create_log}"
    );
    assert!(
        create_log.contains("feat: custom title"),
        "first create should use custom title: {create_log}"
    );
    assert!(
        create_log.contains("custom body text"),
        "first create should use custom body: {create_log}"
    );

    // Verify only one PR was created
    assert_eq!(
        create_log.matches("pr create").count(),
        1,
        "only one PR should have been created"
    );

    // Second delivery: update PR with title-only edit
    let _preview2 = preview_on_github_with(
        &root,
        &fake,
        &rewrites,
        "checkout",
        &["--name", "feat: updated title only"],
    );
    let _applied2 = deliver_on_github_with(
        &root,
        &fake,
        &rewrites,
        "checkout",
        &["--name", "feat: updated title only"],
    );

    // Check that pr edit was called with --title but NOT --body
    let log2 = fake.log();
    let last_edit = log2.lines().rfind(|l| l.contains("pr edit")).unwrap_or("");
    assert!(
        last_edit.contains("--title"),
        "title-only edit should pass --title: {last_edit}"
    );
    assert!(
        last_edit.contains("feat: updated title only"),
        "title-only edit should have the new title: {last_edit}"
    );
    assert!(
        !last_edit.contains("--body"),
        "title-only edit should NOT pass --body: {last_edit}"
    );

    // Third delivery: update PR with body-only edit
    let _preview3 = preview_on_github_with(
        &root,
        &fake,
        &rewrites,
        "checkout",
        &["--body", "updated body only"],
    );
    let _applied3 = deliver_on_github_with(
        &root,
        &fake,
        &rewrites,
        "checkout",
        &["--body", "updated body only"],
    );

    let log3 = fake.log();
    let last_edit3 = log3.lines().rfind(|l| l.contains("pr edit")).unwrap_or("");
    assert!(
        last_edit3.contains("--body"),
        "body-only edit should pass --body: {last_edit3}"
    );
    assert!(
        last_edit3.contains("updated body only"),
        "body-only edit should have the new body: {last_edit3}"
    );
    assert!(
        !last_edit3.contains("--title"),
        "body-only edit should NOT pass --title: {last_edit3}"
    );

    // Fourth delivery: update PR with both fields
    let _applied4 = deliver_on_github_with(
        &root,
        &fake,
        &rewrites,
        "checkout",
        &["--name", "feat: new title", "--body", "new body"],
    );

    let log4 = fake.log();
    let last_edit4 = log4.lines().rfind(|l| l.contains("pr edit")).unwrap_or("");
    assert!(
        last_edit4.contains("--title"),
        "both-fields edit should pass --title: {last_edit4}"
    );
    assert!(
        last_edit4.contains("feat: new title"),
        "both-fields edit should have the new title: {last_edit4}"
    );
    assert!(
        last_edit4.contains("--body"),
        "both-fields edit should pass --body: {last_edit4}"
    );
    assert!(
        last_edit4.contains("new body"),
        "both-fields edit should have the new body: {last_edit4}"
    );

    // Fifth delivery: no-op with no metadata flags - should NOT call pr edit
    let prev_before_noop = {
        let log = fake.log();
        log.matches("pr edit").count()
    };
    let _preview5 = preview_on_github(&root, &fake, &rewrites, "checkout");
    let _applied5 = deliver_on_github(&root, &fake, &rewrites, "checkout");

    let log5 = fake.log();
    let edit_count_after_noop = log5.matches("pr edit").count();
    assert!(
        edit_count_after_noop == prev_before_noop,
        "no-op when no metadata flags: no additional pr edit should be called (was {prev_before_noop}, now {edit_count_after_noop})"
    );
}

/// Body file change invalidates fingerprint: changing the content of a body
/// file and re-applying should be rejected by the fingerprint gate.
#[test]
fn body_file_change_invalidates_fingerprint() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");

    // Write initial body file
    let body_md = root.join("body.md");
    std::fs::write(&body_md, "original content\n").unwrap();

    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Preview with body file to get fingerprint
    let preview = preview_on_github_with(
        &root,
        &fake,
        &rewrites,
        "checkout",
        &["--name", "feat", "--body", "./body.md"],
    );
    let fp = preview["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();

    // Modify the body file content AFTER the preview
    std::fs::write(&body_md, "modified content\n").unwrap();

    // Apply with old fingerprint should fail (drift detection)
    ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--fingerprint",
            &fp,
            "--name",
            "feat",
            "--body",
            "./body.md",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("drifted"));
}

/// Push-only behavior: local path repo should remain push-only with no PR.
#[test]
fn push_only_behavior() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");

    // Delivery with no GitHub remote should be push-only
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
    let repo = &preview["repos"][0];
    assert_eq!(
        repo["action"], "push_only",
        "local path repo should be push-only"
    );
    assert!(
        repo["pr_url"].is_null(),
        "push-only repo should have no pr_url"
    );
    assert!(
        repo["pr_title"].is_null(),
        "push-only repo should have no pr_title"
    );
    assert!(
        repo["pr_body"].is_null(),
        "push-only repo should have no pr_body"
    );
}

/// Legacy defaults: when no --name/--body supplied, new PR creation uses the
/// historical default title and body.
#[test]
fn legacy_defaults() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Preview with no metadata flags - action should be new_pr
    let output = ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args(["feature", "deliver", "checkout", "--preview", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    let preview = &value["preview"];
    let repo = &preview["repos"][0];
    assert_eq!(repo["action"], "new_pr", "GitHub repo should create a PR");
    // When no metadata is supplied, pr_title and pr_body are absent (null)
    assert!(
        repo["pr_title"].is_null(),
        "pr_title should be absent when no --name is supplied"
    );
    assert!(
        repo["pr_body"].is_null(),
        "pr_body should be absent when no --body is supplied"
    );

    // Apply with fingerprint - PR should be created with historical defaults
    let fp = preview["fingerprint"].as_str().unwrap().to_owned();
    let _output2 = ivar_on_github(&fake, &rewrites)
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

    // The fake gh log should show pr create with default title and body
    let log = fake.log();
    assert!(
        log.contains("pr create --base main --head checkout"),
        "gh pr create should be called: {log}"
    );
    assert!(
        log.contains("--title checkout"),
        "default title should be the feature name: {log}"
    );
    assert!(
        log.contains("--body Part of feature `checkout`."),
        "default body should be used: {log}"
    );
}
