//! Pull-request title/body scope, body files, edits and land-mode refusals,
//! delivered in-process.

use super::fixture::*;
use super::*;

fn repo_of<'a>(value: &'a serde_json::Value, repo: &str) -> &'a serde_json::Value {
    value["preview"]["repos"]
        .as_array()
        .expect("repos is an array")
        .iter()
        .find(|r| r["repo"] == repo)
        .expect("repo found")
}

fn scoped(repo: &str, title: Option<&str>, body: Option<&str>) -> RepoMetadataOverride {
    RepoMetadataOverride {
        repo: repo.to_owned(),
        metadata: PullRequestMetadata {
            title: title.map(str::to_owned),
            body: body.map(str::to_owned),
            draft: None,
        },
    }
}

fn last_edit(log: &str) -> &str {
    log.lines().rfind(|l| l.contains("pr edit")).unwrap_or("")
}

/// Global custom creation: `--name` and `--body` as global flags create a PR
/// with those values as title and body.
#[test]
fn global_custom_creation() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    let metadata = metadata_input(Some("feat: global title"), Some("global body inline"), None);

    let value = preview_json(&root, metadata.clone());
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

    let value2 = deliver_json(&root, metadata);
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
    let (_guard, root, fake) = on_github(&["api", "web"]);
    let _gh = fake_gh_on_this_thread(&fake);

    let value = preview_json(
        &root,
        DeliverInput {
            repo_overrides: vec![scoped(
                "api",
                Some("feat(api): custom title"),
                Some("custom api body"),
            )],
            ..metadata_input(Some("feat: global title"), Some("global body"), None)
        },
    );

    let api_repo = repo_of(&value, "api");
    assert_eq!(
        api_repo["pr_title"], "feat(api): custom title",
        "api should have custom title from --repo"
    );
    assert_eq!(
        api_repo["pr_body"], "custom api body",
        "api should have custom body from --repo"
    );
    let web_repo = repo_of(&value, "web");
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
    let (_guard, root, fake) = on_github(&["api", "web"]);
    let _gh = fake_gh_on_this_thread(&fake);

    let value = preview_json(
        &root,
        DeliverInput {
            repo_overrides: vec![scoped("api", Some("feat(api): custom title"), None)],
            ..metadata_input(Some("feat: global title"), Some("global body"), None)
        },
    );

    let api_repo = repo_of(&value, "api");
    assert_eq!(
        api_repo["pr_title"], "feat(api): custom title",
        "api title should come from --repo override"
    );
    assert_eq!(
        api_repo["pr_body"], "global body",
        "api body should be inherited from global --body"
    );
    let web_repo = repo_of(&value, "web");
    assert_eq!(
        web_repo["pr_title"], "feat: global title",
        "web title should be inherited from global --name"
    );
    assert_eq!(
        web_repo["pr_body"], "global body",
        "web body should be inherited from global --body"
    );

    // Global title only, web override supplies body only.
    let value2 = preview_json(
        &root,
        DeliverInput {
            repo_overrides: vec![scoped("web", None, Some("custom web body"))],
            ..metadata_input(Some("feat: global title"), None, None)
        },
    );

    let api_repo2 = repo_of(&value2, "api");
    assert_eq!(
        api_repo2["pr_title"], "feat: global title",
        "api title should be inherited from global --name"
    );
    assert!(
        api_repo2["pr_body"].is_null(),
        "api body should be absent (no global body, no api override)"
    );
    let web_repo2 = repo_of(&value2, "web");
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
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    let body_md = root.join("body.md");
    std::fs::write(&body_md, "Content from file\n").unwrap();
    std::fs::write(root.join("body.txt"), "Content from txt\n").unwrap();
    let previewed_body = |body: &str| {
        preview_json(&root, metadata_input(Some("feat"), Some(body), None))["preview"]["repos"][0]
            .clone()
    };

    let repo = previewed_body("inline body text");
    assert_eq!(repo["pr_title"], "feat", "title should be set");
    assert_eq!(
        repo["pr_body"], "inline body text",
        "inline body should be stored"
    );
    assert_eq!(
        previewed_body("./body.md")["pr_body"],
        "Content from file\n",
        "file body ./body.md should resolve to file content"
    );
    assert_eq!(
        previewed_body("./body.txt")["pr_body"],
        "Content from txt\n",
        "file body ./body.txt should resolve to file content"
    );
    assert_eq!(
        previewed_body("body.md")["pr_body"],
        "body.md",
        "non-prefixed body.md should be treated as inline text"
    );
    // The argument is path-shaped, so inline text is never what was meant.
    assert_eq!(
        previewed_body(body_md.as_str())["pr_body"],
        "Content from file\n",
        "an absolute .md path should resolve to file content"
    );
}

/// Body file change invalidates fingerprint: changing the content of a body
/// file and re-applying should be rejected by the fingerprint gate.
#[test]
fn body_file_change_invalidates_fingerprint() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    let body_md = root.join("body.md");
    std::fs::write(&body_md, "original content\n").unwrap();
    let from_file = metadata_input(Some("feat"), Some("./body.md"), None);
    let fp = preview_json(&root, from_file.clone())["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();

    std::fs::write(&body_md, "modified content\n").unwrap();

    let failure = deliver(
        &Ctx::new(root.clone()),
        DeliverInput {
            preview: false,
            fingerprint: Some(fp),
            ..from_file
        },
    )
    .unwrap_err();
    let text = failure_text(&failure);
    assert!(text.contains("drifted"), "{text}");
}

/// Metadata plus land rejection: passing metadata flags with --land is rejected.
#[test]
fn metadata_plus_land_rejection() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    let ctx = Ctx::new(root.clone());

    for (title, body) in [
        (Some("feat: should fail"), None),
        (None, Some("./body.txt")),
    ] {
        let failure = deliver(
            &ctx,
            DeliverInput {
                land: true,
                ..metadata_input(title, body, None)
            },
        )
        .unwrap_err();
        let text = failure_text(&failure);
        assert!(text.contains("cannot be used in land mode"), "{text}");
    }
}

/// Push-only behavior: local path repo should remain push-only with no PR.
#[test]
fn push_only_behavior() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);

    let value = preview_json(&root, preview_input("checkout"));

    let repo = &value["preview"]["repos"][0];
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

/// Existing PR title-only/body-only/both/no-op edits.
#[test]
fn existing_pr_edits() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);

    let applied = deliver_json(
        &root,
        metadata_input(Some("feat: custom title"), Some("custom body text"), None),
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
    assert_eq!(
        create_log.matches("pr create").count(),
        1,
        "only one PR should have been created"
    );

    deliver_json(
        &root,
        metadata_input(Some("feat: updated title only"), None, None),
    );
    let log2 = fake.log();
    let last_edit2 = last_edit(&log2);
    assert!(
        last_edit2.contains("--title"),
        "title-only edit should pass --title: {last_edit2}"
    );
    assert!(
        last_edit2.contains("feat: updated title only"),
        "title-only edit should have the new title: {last_edit2}"
    );
    assert!(
        !last_edit2.contains("--body"),
        "title-only edit should NOT pass --body: {last_edit2}"
    );
    // `gh pr edit` takes the PR as a positional argument; real `gh` rejects
    // `--url`, while the fake accepts any flag.
    assert!(
        !last_edit2.contains("--url"),
        "`gh pr edit` has no --url flag; the PR must be positional: {last_edit2}"
    );

    deliver_json(&root, metadata_input(None, Some("updated body only"), None));
    let log3 = fake.log();
    let last_edit3 = last_edit(&log3);
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

    deliver_json(
        &root,
        metadata_input(Some("feat: new title"), Some("new body"), None),
    );
    let log4 = fake.log();
    let last_edit4 = last_edit(&log4);
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

    let edits_before_noop = fake.log().matches("pr edit").count();
    deliver_json(&root, preview_input("checkout"));
    let edits_after_noop = fake.log().matches("pr edit").count();
    assert_eq!(
        edits_after_noop, edits_before_noop,
        "no-op when no metadata flags: no additional pr edit should be called"
    );
}
