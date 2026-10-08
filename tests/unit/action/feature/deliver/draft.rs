//! Draft pull-request creation, conversion and fingerprint contracts,
//! delivered in-process through the fake `gh`.

use super::fixture::*;
use super::*;

#[test]
fn partial_failure_is_reported_and_pr_not_reverted() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let fake = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&fake);
    as_github_remotes(&root);
    deliver_json(&root, preview_input("checkout"));

    // Second delivery with --draft and --name: edit + conversion, with the
    // conversion forced to fail in the fake.
    let draft_retitled = DeliverInput {
        global_metadata: PullRequestMetadata {
            title: Some("new title".to_owned()),
            body: None,
            draft: Some(true),
        },
        ..preview_input("checkout")
    };
    let _failing_ready =
        redirect_on_this_thread("gh", fake_gh_command(&fake).env("GH_FAKE_READY_FAIL", "1"));
    let applied = deliver_json_expecting_warnings(&root, draft_retitled);

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
    assert!(
        log.contains("pr ready --undo"),
        "should show conversion attempt"
    );
}

fn draft() -> DeliverInput {
    metadata_input(None, None, Some(true))
}

fn draft_of<'a>(value: &'a serde_json::Value, repo: &str) -> &'a serde_json::Value {
    &value["preview"]["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["repo"] == repo)
        .unwrap()["draft"]
}

/// An existing ready PR is converted with `gh pr ready --undo <url>`.
#[test]
fn existing_ready_pr_conversion_uses_pr_ready_undo() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    deliver_json(&root, preview_input("checkout"));

    let preview = preview_json(&root, draft());
    assert_eq!(
        preview["preview"]["repos"][0]["draft"], "convert_to_draft",
        "existing ready PR should plan convert_to_draft"
    );

    deliver_json(&root, draft());
    let log = fake.log();
    assert!(
        log.contains("pr ready --undo https://github.com/acme/pull/1"),
        "should call pr ready --undo on the PR URL: {log}"
    );
    assert_eq!(
        log.matches("pr create").count(),
        1,
        "only the initial pr create should appear: {log}"
    );
}

/// A PR that disappears after fingerprint validation is recreated as draft,
/// never exposed as ready before a follow-up conversion.
#[test]
fn disappearing_ready_pr_is_recreated_as_draft_atomically() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    deliver_json(&root, preview_input("checkout"));
    let fingerprint = preview_json(&root, draft())["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    std::fs::write(&fake.log, "").unwrap();

    let _vanishing = redirect_on_this_thread(
        "gh",
        fake_gh_command(&fake).env("GH_FAKE_LIST_EMPTY_AFTER_FIRST", "1"),
    );
    let report = deliver(
        &Ctx::new(root.clone()),
        DeliverInput {
            preview: false,
            fingerprint: Some(fingerprint),
            ..draft()
        },
    )
    .unwrap();

    assert!(report.is_clean(), "{:?}", report.warnings);
    let log = fake.log();
    let create = log.lines().find(|line| line.contains("pr create")).unwrap();
    assert!(
        create.contains("--draft"),
        "fallback creation must be atomic: {log}"
    );
    assert!(
        !log.contains("pr ready --undo"),
        "new draft needs no conversion: {log}"
    );
}

/// Global `--draft` applies to all repos in a two-repo delivery.
#[test]
fn global_draft_creates_both_as_draft() {
    let (_guard, root, fake) = on_github(&["api", "web"]);
    let _gh = fake_gh_on_this_thread(&fake);

    let preview = preview_json(&root, draft());
    for repo in preview["preview"]["repos"].as_array().unwrap() {
        assert_eq!(
            repo["draft"], "create_as_draft",
            "global --draft should apply to {}",
            repo["repo"]
        );
    }

    let applied = deliver_json(&root, draft());
    let log = fake.log();
    let create_count = log.matches("pr create").count();
    assert_eq!(create_count, 2, "two repos should each create a PR: {log}");
    assert!(
        log.matches("--draft").count() >= 2,
        "both creates should use --draft: {log}"
    );

    let pr_url_of = |repo: &str| {
        applied["preview"]["repos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["repo"] == repo)
            .unwrap()["pr_url"]
            .clone()
    };
    assert_eq!(pr_url_of("api"), "https://github.com/acme/pull/1");
    assert_eq!(pr_url_of("web"), "https://github.com/acme/pull/2");
}

/// Scoped `--draft` applies only to the named repo; the other is untouched.
#[test]
fn scoped_draft_applies_only_to_named_repo() {
    let (_guard, root, fake) = on_github(&["api", "web"]);
    let _gh = fake_gh_on_this_thread(&fake);
    let api_draft = DeliverInput {
        repo_overrides: vec![RepoMetadataOverride {
            repo: "api".to_owned(),
            metadata: PullRequestMetadata {
                draft: Some(true),
                ..PullRequestMetadata::default()
            },
        }],
        ..preview_input("checkout")
    };

    let preview = preview_json(&root, api_draft.clone());
    assert_eq!(draft_of(&preview, "api"), "create_as_draft");
    assert!(
        draft_of(&preview, "web").is_null(),
        "web should have no draft action"
    );

    deliver_json(&root, api_draft);
    let log = fake.log();
    assert_eq!(log.matches("pr create").count(), 2);
    let draft_creates = log
        .lines()
        .filter(|l| l.contains("pr create") && l.contains("--draft"))
        .count();
    assert_eq!(
        draft_creates, 1,
        "only api's pr create should use --draft: {log}"
    );
}

/// An already-draft PR receives no readiness command when `--draft` is set.
#[test]
fn already_draft_pr_skips_conversion() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    deliver_json(&root, preview_input("checkout"));

    // Simulates someone converting the PR back to draft on GitHub.
    fake.set_pr_draft_state("https://github.com/acme/pull/1", true);

    let preview = preview_json(&root, draft());
    assert!(
        preview["preview"]["repos"][0]["draft"].is_null(),
        "already-draft PR should have no draft action"
    );

    deliver_json(&root, draft());
    let log = fake.log();
    assert!(
        !log.contains("pr ready"),
        "no pr ready command should appear for an already-draft PR: {log}"
    );
}

/// Metadata update precedes draft conversion; the preview shows both actions.
#[test]
fn metadata_edit_precedes_conversion() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    deliver_json(&root, preview_input("checkout"));
    let retitled_draft = metadata_input(Some("feat: updated title"), None, Some(true));

    let preview = preview_json(&root, retitled_draft.clone());
    let repo = &preview["preview"]["repos"][0];
    assert_eq!(repo["action"], "update_pr");
    assert_eq!(repo["draft"], "convert_to_draft");
    assert_eq!(repo["pr_title"], "feat: updated title");

    deliver_json(&root, retitled_draft);
    let log = fake.log();
    assert!(
        log.contains("pr edit"),
        "metadata edit should run before conversion: {log}"
    );
    assert!(
        log.contains("pr ready --undo"),
        "conversion should run after metadata edit: {log}"
    );
    let edit_pos = log.find("pr edit").unwrap();
    let ready_pos = log.find("pr ready").unwrap();
    assert!(
        edit_pos < ready_pos,
        "pr edit should precede pr ready --undo: {log}"
    );

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
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    let bare = root.join(".ivar/repos/api/.bare");
    fake.set_existing_draft_pr(&bare, "checkout", "https://github.com/acme/pull/42", "main");
    fake.set_pr_draft_state("https://github.com/acme/pull/42", false);

    let preview = preview_json(&root, draft());
    assert_eq!(
        preview["preview"]["repos"][0]["draft"], "convert_to_draft",
        "seeded ready PR should plan conversion"
    );

    deliver_json(&root, draft());
    let log = fake.log();
    assert!(
        log.contains("pr ready --undo https://github.com/acme/pull/42"),
        "should convert the seeded PR: {log}"
    );
    assert_eq!(
        log.matches("pr create").count(),
        0,
        "no new PR should be created: {log}"
    );
}

#[test]
fn conversion_is_idempotent() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    deliver_json(&root, preview_input("checkout"));
    deliver_json(&root, draft());

    let preview = preview_json(&root, draft());
    assert!(
        preview["preview"]["repos"][0]["draft"].is_null(),
        "already-converted-to-draft PR should not plan conversion again: {:#?}",
        preview["preview"]["repos"][0]
    );
}

/// Omitting `--draft` never invokes a readiness command.
#[test]
fn no_draft_flag_skips_readiness_command() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    deliver_json(&root, preview_input("checkout"));

    let preview = preview_json(&root, preview_input("checkout"));
    assert!(
        preview["preview"]["repos"][0]["draft"].is_null(),
        "no draft flag should produce no draft action"
    );

    deliver_json(&root, preview_input("checkout"));
    let log = fake.log();
    assert!(
        !log.contains("pr ready"),
        "no pr ready command should appear without --draft: {log}"
    );
    assert_eq!(log.matches("pr create").count(), 1);
}

/// Changing the remote PR's draft state between preview and apply
/// invalidates the fingerprint (via the `draft` field in DeliveryRepo).
#[test]
fn remote_draft_state_change_invalidates_fingerprint() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    deliver_json(&root, preview_input("checkout"));
    let fp = preview_json(&root, draft())["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();

    fake.set_pr_draft_state("https://github.com/acme/pull/1", true);

    let failure = deliver(
        &Ctx::new(root.clone()),
        DeliverInput {
            preview: false,
            fingerprint: Some(fp),
            ..draft()
        },
    )
    .unwrap_err();
    let text = failure_text(&failure);
    assert!(
        text.contains("drifted"),
        "stale fingerprint should be rejected: {text}"
    );
}

/// Fingerprint changes when the draft action differs between create and convert.
#[test]
fn fingerprint_differs_between_create_and_convert() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);

    let preview_create = preview_json(&root, draft());
    let fp_create = preview_create["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        preview_create["preview"]["repos"][0]["draft"],
        "create_as_draft"
    );

    deliver_json(&root, preview_input("checkout"));

    let preview_convert = preview_json(&root, draft());
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
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    deliver_json(&root, preview_input("checkout"));

    let preview = preview_json(&root, preview_input("checkout"));
    assert!(
        preview["preview"]["repos"][0]["draft"].is_null(),
        "no draft flag should produce no draft action"
    );

    deliver_json(&root, preview_input("checkout"));
    let log = fake.log();
    assert!(
        !log.contains("pr ready"),
        "no pr ready command without --draft: {log}"
    );
    assert_eq!(log.matches("pr create").count(), 1);
}

/// --draft with --land is rejected by metadata validation.
#[test]
fn draft_with_land_is_rejected() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);

    deliver(
        &Ctx::new(root.clone()),
        DeliverInput {
            land: true,
            ..draft()
        },
    )
    .unwrap_err();
}
