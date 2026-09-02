//! Draft preview, fingerprint, help, and validation contracts.

use crate::common::{FakeGh, hall_root};
use crate::support::{
    approve_through_plan, as_github_remotes, deliver_on_github, preview_on_github,
    preview_on_github_with, setup_deliver_hall,
};

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

/// `feature deliver --help` documents `--draft`, its positional repository
/// scope (global before `--repo`, scoped after `--repo <name>`), and its
/// conflict with `--land`.
#[test]
fn draft_help_documents_scope_and_land_conflict() {
    let output = crate::common::ivar()
        .args(["feature", "deliver", "--help"])
        .output()
        .expect("help runs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--draft"),
        "--draft flag must appear in deliver help: {stdout}"
    );
    assert!(
        stdout.contains("--repo"),
        "--repo flag must appear in deliver help: {stdout}"
    );
    assert!(
        stdout.contains("--land"),
        "--land flag must appear in deliver help: {stdout}"
    );

    // The --draft help text must describe scoping: global before --repo, scoped after.
    // Check that the --draft description mentions "global" or "before" and "--repo".
    let draft_section = find_flag_help(&stdout, "draft");
    assert!(
        draft_section.contains("global") || draft_section.contains("before"),
        "--draft help must describe positional scoping (global before --repo): {draft_section}"
    );
    assert!(
        draft_section.contains("--repo"),
        "--draft help must mention --repo scoping: {draft_section}"
    );

    // The --draft help text must describe the conflict with --land.
    assert!(
        draft_section.contains("land") || draft_section.contains("--land"),
        "--draft help must mention the --land conflict: {draft_section}"
    );
}

/// Extract the help text for a specific long flag from `--help` output.
/// Returns the description line(s) for `--<flag_name>`.
fn find_flag_help(help_output: &str, flag_name: &str) -> String {
    let marker = format!("--{flag_name}");
    let lines: Vec<&str> = help_output.lines().collect();
    let mut result = String::new();
    let mut collecting = false;

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.starts_with(&marker) {
            collecting = true;
            result.push_str(trimmed);
            result.push('\n');
            continue;
        }
        if collecting {
            // Stop at the next flag definition or empty section
            if trimmed.starts_with("--") || trimmed.is_empty() {
                break;
            }
            result.push_str(trimmed);
            result.push('\n');
        }
    }
    result
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
