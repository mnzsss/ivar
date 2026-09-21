use crate::common::{FakeGh, hall_root};
use crate::support::{
    approve_through_plan, as_github_remotes, deliver_on_github, deliver_on_github_with,
    preview_on_github, preview_on_github_with, setup_deliver_hall, setup_two_repo_hall,
};

#[test]
fn delivering_a_github_repo_opens_a_pull_request_and_records_its_url() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    let preview = preview_on_github(&root, &fake, &rewrites, "checkout");
    assert_eq!(preview["preview"]["repos"][0]["action"], "new_pr");

    let applied = deliver_on_github(&root, &fake, &rewrites, "checkout");
    assert_eq!(
        applied["preview"]["repos"][0]["pr_url"], "https://github.com/acme/pull/1",
        "the PR URL `gh` printed has to land on the repo"
    );
}

#[test]
fn delivering_again_updates_the_existing_pull_request_instead_of_failing() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    deliver_on_github(&root, &fake, &rewrites, "checkout");

    // Second run: the branch already has a PR, so the action flips to
    // `update_pr` — pushing updates it in place, and nothing is recreated.
    let preview = preview_on_github(&root, &fake, &rewrites, "checkout");
    assert_eq!(preview["preview"]["repos"][0]["action"], "update_pr");

    let applied = deliver_on_github(&root, &fake, &rewrites, "checkout");
    assert_eq!(
        applied["preview"]["repos"][0]["pr_url"], "https://github.com/acme/pull/1",
        "an update keeps reporting the PR it updated"
    );
    assert_eq!(
        fake.log().matches("pr create").count(),
        1,
        "a second `gh pr create` for a branch that already has a PR is the bug"
    );
}

#[test]
fn delivering_a_draft_pr_sets_correct_state() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    let preview = preview_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    assert_eq!(preview["preview"]["repos"][0]["draft"], "create_as_draft");

    let _applied = deliver_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    let log = fake.log();
    assert!(
        log.contains("--draft"),
        "gh pr create should include --draft flag: {log}"
    );

    // Verify state file records draft flag.
    let pr_state = std::fs::read_to_string(&fake.state).unwrap();
    assert!(
        pr_state.ends_with("|1\n") || pr_state.contains("|1\n"),
        "state should hold is_draft=1 at field 10: {pr_state}"
    );
}

#[test]
fn sibling_pull_requests_are_linked_to_each_other() {
    let (_guard, root) = hall_root();
    setup_two_repo_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    deliver_on_github(&root, &fake, &rewrites, "checkout");

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
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Seed a pre-existing ready (non-draft) PR so the preview has something
    // to observe for both the action (update_pr) and draft action
    // (convert_to_draft).
    let bare = root.join(".ivar/repos/api/.bare");
    fake.set_existing_draft_pr(&bare, "checkout", "https://github.com/acme/pull/1", "main");
    fake.set_pr_draft_state("https://github.com/acme/pull/1", false);

    let preview = preview_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    assert_eq!(preview["preview"]["repos"][0]["action"], "update_pr");
    assert_eq!(
        preview["preview"]["repos"][0]["draft"], "convert_to_draft",
        "an existing ready PR should be converted to draft"
    );

    // Exactly one `gh pr list` per repo — the observation for both action and
    // draft is a single call, not two independent round-trips.
    assert_eq!(
        fake.log().matches("pr list").count(),
        1,
        "preview should observe the open PR exactly once, not once per decision point"
    );
}

#[test]
fn apply_reports_the_pull_request_it_created_and_updated() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    let created = deliver_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    assert_eq!(
        created["pushes"][0]["pr"],
        serde_json::json!({"number": 1, "url": "https://github.com/acme/pull/1", "draft": true})
    );

    let updated = deliver_on_github(&root, &fake, &rewrites, "checkout");
    assert_eq!(updated["pushes"][0]["pr"]["number"], 1);
    assert_eq!(
        updated["pushes"][0]["pr"]["url"],
        "https://github.com/acme/pull/1"
    );
    assert_eq!(updated["pushes"][0]["pr"]["draft"], true);
}

#[test]
fn redelivering_the_same_title_and_body_does_not_edit_the_pull_request() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);
    let metadata = ["--name", "feat: title", "--body", "the body"];

    deliver_on_github_with(&root, &fake, &rewrites, "checkout", &metadata);
    deliver_on_github_with(&root, &fake, &rewrites, "checkout", &metadata);

    assert_eq!(fake.log().matches("pr edit").count(), 0, "{}", fake.log());
}

#[test]
fn redelivering_siblings_does_not_repeat_the_comment() {
    let (_guard, root) = hall_root();
    setup_two_repo_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    deliver_on_github(&root, &fake, &rewrites, "checkout");
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    let log = fake.log();
    assert_eq!(log.matches("pr comment").count(), 2, "{log}");
    assert!(!log.contains("api graphql"), "{log}");
}

#[test]
fn a_sibling_comment_differing_only_in_line_endings_and_whitespace_is_not_edited() {
    let (_guard, root) = hall_root();
    setup_two_repo_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    deliver_on_github(&root, &fake, &rewrites, "checkout");
    fake.set_comment(
        "https://github.com/acme/pull/1",
        "## Sibling PRs:\r\n\r\nThis PR is part of feature delivery alongside:\r\n\r\n- https://github.com/acme/pull/2\r\n  \r\n",
    );
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    let log = fake.log();
    assert_eq!(log.matches("pr comment").count(), 2, "{log}");
    assert!(!log.contains("api graphql"), "{log}");
}

#[test]
fn a_stale_sibling_comment_is_edited_in_place() {
    let (_guard, root) = hall_root();
    setup_two_repo_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    deliver_on_github(&root, &fake, &rewrites, "checkout");
    fake.set_comment(
        "https://github.com/acme/pull/1",
        "## Sibling PRs:\n\n- stale",
    );
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    let log = fake.log();
    assert_eq!(log.matches("pr comment").count(), 2, "{log}");
    assert_eq!(log.matches("api graphql").count(), 1, "{log}");
}

#[test]
fn redelivering_a_new_body_edits_only_the_body() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    deliver_on_github_with(
        &root,
        &fake,
        &rewrites,
        "checkout",
        &["--name", "feat: title", "--body", "old"],
    );
    deliver_on_github_with(
        &root,
        &fake,
        &rewrites,
        "checkout",
        &["--name", "feat: title", "--body", "new"],
    );

    let log = fake.log();
    let edit = log
        .lines()
        .rfind(|line| line.starts_with("pr edit"))
        .unwrap();
    assert!(edit.contains("--body new"), "{log}");
    assert!(!edit.contains("--title"), "{log}");
}

#[test]
fn an_unreadable_pull_request_is_edited_with_every_requested_field() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);
    let metadata = ["--name", "feat: title", "--body", "the body"];

    deliver_on_github_with(&root, &fake, &rewrites, "checkout", &metadata);
    fake.fail_pr_view("title,body");
    deliver_on_github_with(&root, &fake, &rewrites, "checkout", &metadata);

    let log = fake.log();
    let edit = log
        .lines()
        .rfind(|line| line.starts_with("pr edit"))
        .unwrap();
    assert!(edit.contains("--title") && edit.contains("--body"), "{log}");
}

#[test]
fn unreadable_comments_skip_sibling_linking_without_failing_delivery() {
    let (_guard, root) = hall_root();
    setup_two_repo_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    fake.fail_pr_view("comments");
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    let log = fake.log();
    assert_eq!(log.matches("pr comment").count(), 0, "{log}");
    assert!(!log.contains("api graphql"), "{log}");
}

#[test]
fn a_sibling_comment_by_someone_else_is_left_alone() {
    let (_guard, root) = hall_root();
    setup_two_repo_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    fake.add_comment(
        "https://github.com/acme/pull/1",
        "someone",
        "## Sibling PRs:\n\n- stale",
    );
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    let log = fake.log();
    assert_eq!(log.matches("pr comment").count(), 2, "{log}");
    assert!(!log.contains("api graphql"), "{log}");
}

#[test]
fn a_comment_quoting_the_sibling_header_mid_text_is_not_the_sibling_comment() {
    let (_guard, root) = hall_root();
    setup_two_repo_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    fake.add_comment(
        "https://github.com/acme/pull/1",
        "acme",
        "see\n## Sibling PRs:\n\n- stale",
    );
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    let log = fake.log();
    assert_eq!(log.matches("pr comment").count(), 2, "{log}");
    assert!(!log.contains("api graphql"), "{log}");
}

#[test]
fn an_unknown_login_still_links_siblings_once() {
    let (_guard, root) = hall_root();
    setup_two_repo_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    fake.fail_api_user();
    deliver_on_github(&root, &fake, &rewrites, "checkout");
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    let log = fake.log();
    assert_eq!(log.matches("pr comment").count(), 2, "{log}");
    assert!(!log.contains("api graphql"), "{log}");
}

#[test]
fn malformed_pull_request_metadata_is_edited_with_every_requested_field() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);
    let metadata = ["--name", "feat: title", "--body", "the body"];

    deliver_on_github_with(&root, &fake, &rewrites, "checkout", &metadata);
    fake.malform_pr_view("title,body");
    deliver_on_github_with(&root, &fake, &rewrites, "checkout", &metadata);

    let log = fake.log();
    let edit = log
        .lines()
        .rfind(|line| line.starts_with("pr edit"))
        .unwrap();
    assert!(edit.contains("--title") && edit.contains("--body"), "{log}");
}

#[test]
fn a_failing_sibling_comment_edit_does_not_fail_delivery() {
    let (_guard, root) = hall_root();
    setup_two_repo_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    deliver_on_github(&root, &fake, &rewrites, "checkout");
    fake.set_comment(
        "https://github.com/acme/pull/1",
        "## Sibling PRs:\n\n- stale",
    );
    fake.fail_graphql();
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    let log = fake.log();
    assert_eq!(log.matches("api graphql").count(), 1, "{log}");
}
