//! Draft pull-request creation and positional scope.

use crate::common::{FakeGh, hall_root};
use crate::support::{
    approve_through_plan, as_github_remotes, deliver_on_github, deliver_on_github_with,
    preview_on_github_with, setup_deliver_hall, setup_two_repo_hall,
};

/// `gh pr create --draft` is invoked exactly once for a new draft PR.
#[test]
fn new_pr_creation_uses_gh_draft_flag() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Preview with --draft so the intent is resolved.
    let preview = preview_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    assert_eq!(preview["preview"]["repos"][0]["draft"], "create_as_draft");

    let applied = deliver_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    assert_eq!(
        applied["preview"]["repos"][0]["pr_url"],
        "https://github.com/acme/pull/1"
    );
    let log = fake.log();
    assert!(
        log.contains("--draft"),
        "gh pr create should include --draft flag: {log}"
    );
    assert_eq!(
        log.matches("pr create").count(),
        1,
        "exactly one pr create call expected"
    );
}

/// An existing ready PR is converted with `gh pr ready --undo <url>`.
#[test]
fn existing_ready_pr_conversion_uses_pr_ready_undo() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Create the initial (ready) PR without --draft.
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    // Second delivery with --draft triggers conversion.
    let preview = preview_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    assert_eq!(
        preview["preview"]["repos"][0]["draft"], "convert_to_draft",
        "existing ready PR should plan convert_to_draft"
    );

    let _applied = deliver_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    let log = fake.log();
    assert!(
        log.contains("pr ready --undo https://github.com/acme/pull/1"),
        "should call pr ready --undo on the PR URL: {log}"
    );
    // No second pr create — the PR already exists.
    assert_eq!(
        log.matches("pr create").count(),
        1,
        "only the initial pr create should appear: {log}"
    );
}

/// A PR that disappears after fingerprint validation is recreated as draft,
/// never exposed as ready before a follow-up conversion.
#[test]
fn disappearing_ready_pr_is_recreated_as_draft_atomically() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    deliver_on_github(&root, &fake, &rewrites, "checkout");
    let fingerprint = preview_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"])
        ["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    std::fs::write(&fake.log, "").unwrap();

    let output = crate::support::ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .env("GH_FAKE_LIST_EMPTY_AFTER_FIRST", "1")
        .args([
            "feature",
            "deliver",
            "checkout",
            "--fingerprint",
            &fingerprint,
            "--draft",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let log = fake.log();
    let create = log.lines().find(|line| line.contains("pr create")).unwrap();
    assert!(
        create.contains("--draft"),
        "fallback creation must be atomic: {log}"
    );
    assert!(
        !log.contains("pr ready --undo"),
        "new draft needs no conversion: {log}"
    );
}
/// Global `--draft` applies to all repos in a two-repo delivery.
#[test]
fn global_draft_creates_both_as_draft() {
    let (_guard, root) = hall_root();
    setup_two_repo_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Global --draft: both repos create as draft.
    let preview = preview_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    let repos = preview["preview"]["repos"].as_array().unwrap();
    for repo in repos {
        assert_eq!(
            repo["draft"], "create_as_draft",
            "global --draft should apply to {}",
            repo["repo"]
        );
    }

    let applied = deliver_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    let log = fake.log();
    // Both repos got pr create --draft.
    let create_count = log.matches("pr create").count();
    assert_eq!(create_count, 2, "two repos should each create a PR: {log}");
    assert!(
        log.matches("--draft").count() >= 2,
        "both creates should use --draft: {log}"
    );

    // Verify the PR URLs are assigned to the correct repos.
    let api_pr = &applied["preview"]["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["repo"] == "api")
        .unwrap()["pr_url"];
    let web_pr = &applied["preview"]["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["repo"] == "web")
        .unwrap()["pr_url"];
    assert_eq!(api_pr, "https://github.com/acme/pull/1");
    assert_eq!(web_pr, "https://github.com/acme/pull/2");
}

/// Scoped `--draft` applies only to the named repo; the other is untouched.
#[test]
fn scoped_draft_applies_only_to_named_repo() {
    let (_guard, root) = hall_root();
    setup_two_repo_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // --repo api --draft: only api creates as draft.
    let preview = preview_on_github_with(
        &root,
        &fake,
        &rewrites,
        "checkout",
        &["--repo", "api", "--draft"],
    );
    let repos = preview["preview"]["repos"].as_array().unwrap();
    let api = repos.iter().find(|r| r["repo"] == "api").unwrap();
    let web = repos.iter().find(|r| r["repo"] == "web").unwrap();
    assert_eq!(api["draft"], "create_as_draft");
    assert!(web["draft"].is_null(), "web should have no draft action");

    let _applied = deliver_on_github_with(
        &root,
        &fake,
        &rewrites,
        "checkout",
        &["--repo", "api", "--draft"],
    );
    let log = fake.log();
    // Both repos create PRs (one with --draft, one without).
    assert_eq!(log.matches("pr create").count(), 2);
    // Count lines containing both "pr create" and "--draft".
    let create_lines: Vec<&str> = log.lines().filter(|l| l.contains("pr create")).collect();
    let draft_creates = create_lines
        .iter()
        .filter(|l| l.contains("--draft"))
        .count();
    assert_eq!(
        draft_creates, 1,
        "only api's pr create should use --draft: {log}"
    );
}
