//! Draft pull-request creation through the CLI.

use crate::common::{FakeGh, hall_root};
use crate::support::{
    approve_through_plan, as_github_remotes, deliver_on_github_with, preview_on_github_with,
    setup_deliver_hall,
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
