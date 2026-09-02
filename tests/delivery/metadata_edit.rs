//! Existing pull-request metadata edits.

use crate::common::{FakeGh, hall_root};
use crate::support::{
    approve_through_plan, as_github_remotes, deliver_on_github, deliver_on_github_with,
    preview_on_github, preview_on_github_with, setup_deliver_hall,
};

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

    // `gh pr edit` takes the PR as a positional argument; there is no `--url`
    // flag. Passing one makes real `gh` exit with `unknown flag: --url`, so
    // every metadata update against an existing PR fails while the fake --
    // which accepts any flag -- still reports success.
    assert!(
        !last_edit.contains("--url"),
        "`gh pr edit` has no --url flag; the PR must be positional: {last_edit}"
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
