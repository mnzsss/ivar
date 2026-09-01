use crate::common::{FakeGh, hall_root};
use crate::support::{
    approve_through_plan, as_github_remotes, deliver_on_github, preview_on_github,
    setup_deliver_hall, setup_two_repo_hall,
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
