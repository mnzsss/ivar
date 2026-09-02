//! Pull-request metadata validation and push-only behavior.

use crate::common::{FakeGh, hall_root, ivar};
use crate::support::{approve_through_plan, as_github_remotes, ivar_on_github, setup_deliver_hall};
use predicates::prelude::*;

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
