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
