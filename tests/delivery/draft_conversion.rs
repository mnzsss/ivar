//! Existing pull-request conversion and failure behavior.

use crate::common::{FakeGh, hall_root};
use crate::support::{
    approve_through_plan, as_github_remotes, deliver_on_github, deliver_on_github_with,
    preview_on_github_with, setup_deliver_hall,
};

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
/// An already-draft PR receives no readiness command when `--draft` is set.
#[test]
fn already_draft_pr_skips_conversion() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Create the PR first (without --draft, so it's ready).
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    // Now mark it as draft in the fake state (simulating someone clicking
    // "Ready for review" → "Convert to draft" on GitHub).
    fake.set_pr_draft_state("https://github.com/acme/pull/1", true);

    // Preview with --draft: the PR is already draft, so no conversion planned.
    let preview = preview_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    assert!(
        preview["preview"]["repos"][0]["draft"].is_null(),
        "already-draft PR should have no draft action"
    );

    let _applied = deliver_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    let log = fake.log();
    assert!(
        !log.contains("pr ready"),
        "no pr ready command should appear for an already-draft PR: {log}"
    );
}
/// Metadata update precedes draft conversion; the preview shows both actions.
#[test]
fn metadata_edit_precedes_conversion() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Create the initial (ready) PR.
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    // Second delivery: metadata update + conversion.
    let preview = preview_on_github_with(
        &root,
        &fake,
        &rewrites,
        "checkout",
        &["--draft", "--name", "feat: updated title"],
    );
    let repo = &preview["preview"]["repos"][0];
    assert_eq!(repo["action"], "update_pr");
    assert_eq!(repo["draft"], "convert_to_draft");
    assert_eq!(repo["pr_title"], "feat: updated title");

    let _applied = deliver_on_github_with(
        &root,
        &fake,
        &rewrites,
        "checkout",
        &["--draft", "--name", "feat: updated title"],
    );
    let log = fake.log();
    // Both edit and ready --undo should appear.
    assert!(
        log.contains("pr edit"),
        "metadata edit should run before conversion: {log}"
    );
    assert!(
        log.contains("pr ready --undo"),
        "conversion should run after metadata edit: {log}"
    );
    // Edit must come before ready in the log.
    let edit_pos = log.find("pr edit").unwrap();
    let ready_pos = log.find("pr ready").unwrap();
    assert!(
        edit_pos < ready_pos,
        "pr edit should precede pr ready --undo: {log}"
    );

    // Verify draft flag is preserved in the fake state after metadata edit.
    let pr_state = std::fs::read_to_string(&fake.state).unwrap();
    assert!(
        pr_state.contains("|1\n") || pr_state.ends_with("|1"),
        "draft flag should be preserved after metadata edit: {pr_state}"
    );
    assert!(
        pr_state.contains("feat: updated title"),
        "title should be updated in fake state: {pr_state}"
    );
}
/// Seed a ready PR in the fake, then deliver with --draft: preview should
/// show convert_to_draft and apply should call pr ready --undo.
#[test]
fn seeded_ready_pr_gets_converted_to_draft() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Seed a pre-existing ready PR (not draft).
    let bare = root.join(".ivar/repos/api/.bare");
    fake.set_existing_draft_pr(&bare, "checkout", "https://github.com/acme/pull/42", "main");
    // Set it to ready (not draft).
    fake.set_pr_draft_state("https://github.com/acme/pull/42", false);

    // Preview with --draft.
    let preview = preview_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    assert_eq!(
        preview["preview"]["repos"][0]["draft"], "convert_to_draft",
        "seeded ready PR should plan conversion"
    );

    let _applied = deliver_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    let log = fake.log();
    assert!(
        log.contains("pr ready --undo https://github.com/acme/pull/42"),
        "should convert the seeded PR: {log}"
    );
    // No new PR creation — one already existed.
    assert_eq!(
        log.matches("pr create").count(),
        0,
        "no new PR should be created: {log}"
    );
}

#[test]
fn conversion_is_idempotent() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Create a ready PR.
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    // Deliver with --draft: conversions to draft.
    let _applied = deliver_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);

    // Now preview again: it should NOT plan convert_to_draft.
    let preview = preview_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    assert!(
        preview["preview"]["repos"][0]["draft"].is_null(),
        "already-converted-to-draft PR should not plan conversion again: {:#?}",
        preview["preview"]["repos"][0]
    );
}

#[test]
fn partial_failure_is_reported_and_pr_not_reverted() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Create the initial ready PR.
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    // Second delivery with --draft and --name: edit + conversion.
    // Force conversion failure in the fake.
    let fp = preview_on_github_with(
        &root,
        &fake,
        &rewrites,
        "checkout",
        &["--draft", "--name", "new title"],
    )["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();

    let output = crate::support::ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .env("GH_FAKE_READY_FAIL", "1")
        .args([
            "feature",
            "deliver",
            "checkout",
            "--fingerprint",
            &fp,
            "--draft",
            "--name",
            "new title",
            "--json",
        ])
        .output()
        .unwrap();

    // The process reports a warning attributable to the draft conversion.
    assert!(
        !output.status.success(),
        "delivery with partially failing conversion should warn"
    );

    // Check JSON output.
    let applied: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid json");
    let pr_url = applied["preview"]["repos"][0]["pr_url"].as_str().unwrap();
    assert_eq!(
        pr_url, "https://github.com/acme/pull/1",
        "PR URL should be in the apply JSON"
    );

    // The title edit is NOT reverted (fake state holds new title).
    let pr_state = std::fs::read_to_string(&fake.state).unwrap();
    assert!(
        pr_state.contains("new title"),
        "title should have been updated even if conversion failed"
    );

    // No compensating/rollback command in the log.
    let log = fake.log();
    assert_eq!(
        log.matches("pr edit").count(),
        1,
        "exactly one metadata edit expected (no rollback edit): {log}"
    );
    // No `pr ready` without `--undo` — a bare `pr ready` would mark the PR
    // ready again, undoing the conversion, which is wrong.
    for line in log.lines() {
        if line.contains("pr ready") {
            assert!(
                line.contains("--undo"),
                "pr ready without --undo would mark the PR ready again: {line}"
            );
        }
    }

    // The emitted warning must carry the distinct conversion code.
    let warnings = applied["warnings"].as_array().expect("warnings array");
    let conversion_warnings: Vec<&serde_json::Value> = warnings
        .iter()
        .filter(|w| w["code"] == "deliver.pr_draft_conversion_failed")
        .collect();
    assert_eq!(
        conversion_warnings.len(),
        1,
        "expected exactly one conversion warning with distinct code"
    );

    // Conversion attempt appears.
    assert!(
        log.contains("pr ready --undo"),
        "should show conversion attempt"
    );
}
