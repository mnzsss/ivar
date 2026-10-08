use super::fixture::*;
use super::*;

#[test]
fn pull_request_lookups_run_the_fake_gh_redirected_for_this_thread() {
    let (_guard, root) = hall_root();
    let fake = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&fake);
    fake.set_existing_pr(
        &root,
        "checkout",
        "https://github.com/acme/api/pull/7",
        "main",
        "OPEN",
    );

    let found = crate::action::feature::pull_requests::find_pull_request(&root, "checkout", "open")
        .unwrap();

    assert_eq!(found.map(|pr| pr.number), Some(7));
    assert!(
        fake.log().contains("pr list --head checkout --state open"),
        "the fake must have answered: {}",
        fake.log()
    );
}

#[test]
fn deliver_refuses_a_child_with_the_integrate_command() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    child_of_checkout(&root, "child");

    // Preview and apply refuse identically.
    let failure = deliver(&ctx, preview_input("child")).unwrap_err();
    assert_eq!(failure.code, "deliver.child_requires_integration");
    assert_eq!(
        failure.fix_actions[0].command.as_deref(),
        Some("ivar feature integrate child")
    );
    let failure = deliver(&ctx, apply_input("child", "whatever")).unwrap_err();
    assert_eq!(failure.code, "deliver.child_requires_integration");
}

#[test]
fn github_repo_in_land_mode_creates_no_pull_request() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);
    let fake = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&fake);
    as_github_remotes(&root);

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            global_metadata: PullRequestMetadata::default(),
            repo_overrides: Vec::new(),
            only: Vec::new(),
        },
    )
    .expect("land preview");

    assert_eq!(
        land_preview.value.preview.repos[0].action,
        DeliveryAction::LandOnDefault
    );

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
            global_metadata: PullRequestMetadata::default(),
            repo_overrides: Vec::new(),
            only: Vec::new(),
        },
    )
    .expect("land apply");

    assert!(
        out.value.preview.repos[0].pr_url.is_none(),
        "land mode must not create a PR URL"
    );
}

#[test]
fn only_unprefixed_github_repos_open_pull_requests() {
    use crate::domain::name::{BranchName, RepoName};
    use crate::store::manifest::Repo;
    let main = BranchName::new("main").unwrap();
    let github = Repo::new(
        RepoName::new("api").unwrap(),
        "https://github.com/acme/api",
        main.clone(),
    );
    let hall_local = Repo::new(
        RepoName::new("notes").unwrap(),
        "https://github.com/acme/hall",
        main.clone(),
    )
    .with_ref_prefix("repos/notes/");
    let elsewhere = Repo::new(RepoName::new("web").unwrap(), "/tmp/origins/web", main);

    assert!(super::repos::opens_pull_requests(&github));
    assert!(!super::repos::opens_pull_requests(&hall_local));
    assert!(!super::repos::opens_pull_requests(&elsewhere));
}

fn last_edit(log: &str) -> &str {
    log.lines()
        .rfind(|line| line.starts_with("pr edit"))
        .unwrap()
}

#[test]
fn a_gh_failure_is_not_reported_as_no_pull_request() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let fake = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&fake);
    as_github_remotes(&root);
    fake.fail_pr_list();

    let failure = deliver(&Ctx::new(root.clone()), preview_input("checkout")).unwrap_err();

    let text = failure_text(&failure);
    assert!(text.contains("gh pr list"), "{text}");
}

#[test]
fn a_delivered_pr_is_remembered_and_its_merge_shows_in_status() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    let applied = deliver_json(&root, preview_input("checkout"));
    let url = applied["pushes"][0]["pr"]["url"]
        .as_str()
        .expect("a PR was opened")
        .to_owned();
    fake.set_pr_state(&url, "MERGED");

    let report = crate::action::feature::status::status(
        &Ctx::new(root.clone()),
        crate::action::feature::status::StatusInput {
            feature: "checkout".to_owned(),
            recursive: false,
        },
    )
    .unwrap();
    assert!(report.is_clean(), "{:?}", report.warnings);
    let status = serde_json::to_value(&report).unwrap();

    assert_eq!(status["repos"][0]["pr_url"], url.as_str());
    assert_eq!(status["repos"][0]["pr_state"], "MERGED");
}

#[test]
fn status_reports_a_gh_failure_instead_of_a_pr_state() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    deliver_json(&root, preview_input("checkout"));
    fake.fail_pr_view("url,number,state,mergeCommit,headRefOid,isDraft");

    let report = crate::action::feature::status::status(
        &Ctx::new(root.clone()),
        crate::action::feature::status::StatusInput {
            feature: "checkout".to_owned(),
            recursive: false,
        },
    )
    .unwrap();
    assert!(!report.is_clean());
    let status = serde_json::to_value(&report).unwrap();

    assert!(status["repos"][0].get("pr_state").is_none());
    assert_eq!(status["warnings"][0]["code"], "feature.pr_state_unknown");
}

#[test]
fn delivering_a_draft_pr_sets_correct_state() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    let draft = metadata_input(None, None, Some(true));

    let preview = preview_json(&root, draft.clone());
    assert_eq!(preview["preview"]["repos"][0]["draft"], "create_as_draft");

    deliver_json(&root, draft);
    let log = fake.log();
    assert!(
        log.contains("--draft"),
        "gh pr create should include --draft flag: {log}"
    );

    let pr_state = std::fs::read_to_string(&fake.state).unwrap();
    assert!(
        pr_state.ends_with("|1\n") || pr_state.contains("|1\n"),
        "state should hold is_draft=1 at field 10: {pr_state}"
    );
}

#[test]
fn sibling_pull_requests_are_linked_to_each_other() {
    let (_guard, root, fake) = on_github(&["api", "web"]);
    let _gh = fake_gh_on_this_thread(&fake);

    deliver_json(&root, preview_input("checkout"));

    let log = fake.log();
    assert!(
        log.contains("pr comment https://github.com/acme/pull/1"),
        "the first PR was never told about its sibling: {log}"
    );
    assert!(
        log.contains("pr comment https://github.com/acme/pull/2"),
        "the second PR was never told about its sibling: {log}"
    );
}

/// Preview with `--draft` against a single repo observes the open PR exactly
/// once: both the baseline action and the draft action derive from one
/// `gh pr list` call, not two.
#[test]
fn draft_preview_makes_exactly_one_pr_observation_per_repo() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    let bare = root.join(".ivar/repos/api/.bare");
    fake.set_existing_draft_pr(&bare, "checkout", "https://github.com/acme/pull/1", "main");
    fake.set_pr_draft_state("https://github.com/acme/pull/1", false);

    let preview = preview_json(&root, metadata_input(None, None, Some(true)));
    assert_eq!(preview["preview"]["repos"][0]["action"], "update_pr");
    assert_eq!(
        preview["preview"]["repos"][0]["draft"], "convert_to_draft",
        "an existing ready PR should be converted to draft"
    );

    assert_eq!(
        fake.log().matches("pr list").count(),
        1,
        "preview should observe the open PR exactly once, not once per decision point"
    );
}

#[test]
fn apply_reports_the_pull_request_it_created_and_updated() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);

    let created = deliver_json(&root, metadata_input(None, None, Some(true)));
    assert_eq!(
        created["pushes"][0]["pr"],
        serde_json::json!({"number": 1, "url": "https://github.com/acme/pull/1", "draft": true})
    );

    let updated = deliver_json(&root, preview_input("checkout"));
    assert_eq!(updated["pushes"][0]["pr"]["number"], 1);
    assert_eq!(
        updated["pushes"][0]["pr"]["url"],
        "https://github.com/acme/pull/1"
    );
    assert_eq!(updated["pushes"][0]["pr"]["draft"], true);
}

#[test]
fn redelivering_the_same_title_and_body_does_not_edit_the_pull_request() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    let metadata = metadata_input(Some("feat: title"), Some("the body"), None);

    deliver_json(&root, metadata.clone());
    deliver_json(&root, metadata);

    assert_eq!(fake.log().matches("pr edit").count(), 0, "{}", fake.log());
}

#[test]
fn redelivering_siblings_does_not_repeat_the_comment() {
    let (_guard, root, fake) = on_github(&["api", "web"]);
    let _gh = fake_gh_on_this_thread(&fake);

    deliver_json(&root, preview_input("checkout"));
    deliver_json(&root, preview_input("checkout"));

    let log = fake.log();
    assert_eq!(log.matches("pr comment").count(), 2, "{log}");
    assert!(!log.contains("api graphql"), "{log}");
}

#[test]
fn a_sibling_comment_differing_only_in_line_endings_and_whitespace_is_not_edited() {
    let (_guard, root, fake) = on_github(&["api", "web"]);
    let _gh = fake_gh_on_this_thread(&fake);

    deliver_json(&root, preview_input("checkout"));
    fake.set_comment(
        "https://github.com/acme/pull/1",
        "## Sibling PRs:\r\n\r\nThis PR is part of feature delivery alongside:\r\n\r\n- https://github.com/acme/pull/2\r\n  \r\n",
    );
    deliver_json(&root, preview_input("checkout"));

    let log = fake.log();
    assert_eq!(log.matches("pr comment").count(), 2, "{log}");
    assert!(!log.contains("api graphql"), "{log}");
}

#[test]
fn a_stale_sibling_comment_is_edited_in_place() {
    let (_guard, root, fake) = on_github(&["api", "web"]);
    let _gh = fake_gh_on_this_thread(&fake);

    deliver_json(&root, preview_input("checkout"));
    fake.set_comment(
        "https://github.com/acme/pull/1",
        "## Sibling PRs:\n\n- stale",
    );
    deliver_json(&root, preview_input("checkout"));

    let log = fake.log();
    assert_eq!(log.matches("pr comment").count(), 2, "{log}");
    assert_eq!(log.matches("api graphql").count(), 1, "{log}");
}

#[test]
fn redelivering_a_new_body_edits_only_the_body() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);

    deliver_json(
        &root,
        metadata_input(Some("feat: title"), Some("old"), None),
    );
    deliver_json(
        &root,
        metadata_input(Some("feat: title"), Some("new"), None),
    );

    let log = fake.log();
    let edit = last_edit(&log);
    assert!(edit.contains("--body new"), "{log}");
    assert!(!edit.contains("--title"), "{log}");
}

#[test]
fn an_unreadable_pull_request_is_edited_with_every_requested_field() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    let metadata = metadata_input(Some("feat: title"), Some("the body"), None);

    deliver_json(&root, metadata.clone());
    fake.fail_pr_view("title,body");
    deliver_json(&root, metadata);

    let log = fake.log();
    let edit = last_edit(&log);
    assert!(edit.contains("--title") && edit.contains("--body"), "{log}");
}

#[test]
fn unreadable_comments_skip_sibling_linking_without_failing_delivery() {
    let (_guard, root, fake) = on_github(&["api", "web"]);
    let _gh = fake_gh_on_this_thread(&fake);

    fake.fail_pr_view("comments");
    deliver_json(&root, preview_input("checkout"));

    let log = fake.log();
    assert_eq!(log.matches("pr comment").count(), 0, "{log}");
    assert!(!log.contains("api graphql"), "{log}");
}

#[test]
fn a_sibling_comment_by_someone_else_is_left_alone() {
    let (_guard, root, fake) = on_github(&["api", "web"]);
    let _gh = fake_gh_on_this_thread(&fake);

    fake.add_comment(
        "https://github.com/acme/pull/1",
        "someone",
        "## Sibling PRs:\n\n- stale",
    );
    deliver_json(&root, preview_input("checkout"));

    let log = fake.log();
    assert_eq!(log.matches("pr comment").count(), 2, "{log}");
    assert!(!log.contains("api graphql"), "{log}");
}

#[test]
fn a_comment_quoting_the_sibling_header_mid_text_is_not_the_sibling_comment() {
    let (_guard, root, fake) = on_github(&["api", "web"]);
    let _gh = fake_gh_on_this_thread(&fake);

    fake.add_comment(
        "https://github.com/acme/pull/1",
        "acme",
        "see\n## Sibling PRs:\n\n- stale",
    );
    deliver_json(&root, preview_input("checkout"));

    let log = fake.log();
    assert_eq!(log.matches("pr comment").count(), 2, "{log}");
    assert!(!log.contains("api graphql"), "{log}");
}

#[test]
fn an_unknown_login_still_links_siblings_once() {
    let (_guard, root, fake) = on_github(&["api", "web"]);
    let _gh = fake_gh_on_this_thread(&fake);

    fake.fail_api_user();
    deliver_json(&root, preview_input("checkout"));
    deliver_json(&root, preview_input("checkout"));

    let log = fake.log();
    assert_eq!(log.matches("pr comment").count(), 2, "{log}");
    assert!(!log.contains("api graphql"), "{log}");
}

#[test]
fn malformed_pull_request_metadata_is_edited_with_every_requested_field() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    let metadata = metadata_input(Some("feat: title"), Some("the body"), None);

    deliver_json(&root, metadata.clone());
    fake.malform_pr_view("title,body");
    deliver_json(&root, metadata);

    let log = fake.log();
    let edit = last_edit(&log);
    assert!(edit.contains("--title") && edit.contains("--body"), "{log}");
}

#[test]
fn a_failing_sibling_comment_edit_does_not_fail_delivery() {
    let (_guard, root, fake) = on_github(&["api", "web"]);
    let _gh = fake_gh_on_this_thread(&fake);

    deliver_json(&root, preview_input("checkout"));
    fake.set_comment(
        "https://github.com/acme/pull/1",
        "## Sibling PRs:\n\n- stale",
    );
    fake.fail_graphql();
    deliver_json(&root, preview_input("checkout"));

    let log = fake.log();
    assert_eq!(log.matches("api graphql").count(), 1, "{log}");
}
