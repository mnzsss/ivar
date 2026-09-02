//! Integration tests for draft pull-request behavior.
//!
//! Exercises creation with `--draft`, conversion via `pr ready --undo`,
//! already-draft idempotence, omitted-flag no-op, global/scoped two-repo
//! behavior, metadata before conversion, partial-failure independence,
//! and fingerprint staleness from remote draft-state change.

use crate::common::{FakeGh, hall_root};
use crate::support::{
    approve_through_plan, as_github_remotes, deliver_on_github, deliver_on_github_with,
    preview_on_github, preview_on_github_with, setup_deliver_hall, setup_two_repo_hall,
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

/// Omitting `--draft` never invokes a readiness command.
#[test]
fn no_draft_flag_skips_readiness_command() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Create the initial PR.
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    // Second delivery without --draft: no conversion.
    let preview = preview_on_github(&root, &fake, &rewrites, "checkout");
    assert!(
        preview["preview"]["repos"][0]["draft"].is_null(),
        "no draft flag should produce no draft action"
    );

    let _applied = deliver_on_github(&root, &fake, &rewrites, "checkout");
    let log = fake.log();
    assert!(
        !log.contains("pr ready"),
        "no pr ready command should appear without --draft: {log}"
    );
    // Only one pr create from the initial delivery.
    assert_eq!(log.matches("pr create").count(), 1);
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
}

/// Changing the remote PR's draft state between preview and apply
/// invalidates the fingerprint (via the `draft` field in DeliveryRepo).
#[test]
fn remote_draft_state_change_invalidates_fingerprint() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Create the initial (ready) PR.
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    // Preview with --draft: plans convert_to_draft (ready PR → draft).
    let preview = preview_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    let fp = preview["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();

    // Meanwhile, the remote PR was converted to draft (simulate).
    fake.set_pr_draft_state("https://github.com/acme/pull/1", true);

    // Apply with the old fingerprint: should fail (drift) because
    // existing_pr now returns is_draft=true → no draft action planned.
    let result = crate::support::ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--fingerprint",
            &fp,
            "--draft",
        ])
        .output()
        .unwrap();
    assert!(
        !result.status.success(),
        "should fail with stale fingerprint"
    );
    // The drift error goes to stderr.
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("drifted"),
        "stale fingerprint should be rejected: {stderr}"
    );
}

/// Fingerprint changes when the draft action differs between create and convert.
#[test]
fn fingerprint_differs_between_create_and_convert() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // First preview: new PR, draft → create_as_draft.
    let preview_create = preview_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    let fp_create = preview_create["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        preview_create["preview"]["repos"][0]["draft"],
        "create_as_draft"
    );

    // Create the PR first (without --draft, so it's ready).
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    // Second preview: existing ready PR, draft → convert_to_draft.
    let preview_convert = preview_on_github_with(&root, &fake, &rewrites, "checkout", &["--draft"]);
    let fp_convert = preview_convert["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        preview_convert["preview"]["repos"][0]["draft"],
        "convert_to_draft"
    );

    assert_ne!(
        fp_create, fp_convert,
        "create_as_draft and convert_to_draft must produce different fingerprints"
    );
}

/// Invoking without --draft on an existing ready PR does not run any
/// readiness command (no implicit ready marking).
#[test]
fn no_draft_on_ready_pr_no_implicit_ready_command() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    // Create the initial PR.
    deliver_on_github(&root, &fake, &rewrites, "checkout");

    // Second delivery: no --draft, no metadata.
    let preview = preview_on_github(&root, &fake, &rewrites, "checkout");
    assert!(
        preview["preview"]["repos"][0]["draft"].is_null(),
        "no draft flag should produce no draft action"
    );

    let _applied = deliver_on_github(&root, &fake, &rewrites, "checkout");
    let log = fake.log();
    // No pr ready command whatsoever.
    assert!(
        !log.contains("pr ready"),
        "no pr ready command without --draft: {log}"
    );
    // Exactly one pr create (from the initial delivery).
    assert_eq!(log.matches("pr create").count(), 1);
}

/// --draft with --land is rejected by metadata validation.
#[test]
fn draft_with_land_is_rejected() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");
    let fake = FakeGh::install(&root);
    let rewrites = as_github_remotes(&root);

    crate::support::ivar_on_github(&fake, &rewrites)
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--draft",
            "--land",
            "--preview",
        ])
        .assert()
        .failure()
        .code(2);
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
