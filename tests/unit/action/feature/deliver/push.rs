use super::fixture::*;
use super::*;

#[test]
fn deliver_pushes_the_feature_branch_to_the_remote() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let approved = deliver(&ctx, preview_input("checkout")).unwrap();
    let fingerprint = approved.value.preview.fingerprint.clone();

    let report = deliver(&ctx, apply_input("checkout", &fingerprint)).unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.pushes.len(), 1);
    assert!(report.value.pushes[0].ok);
    assert_eq!(report.value.pushes[0].repo.as_str(), "api");
    // The remote now holds the branch, at the tip that was previewed.
    assert!(remote_ref(&origin_of(&root, "api"), "checkout").is_some());
}

#[test]
fn a_branch_the_remote_already_carries_is_not_reported_as_unpushed() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let approved = deliver(&ctx, preview_input("checkout")).unwrap();
    deliver(
        &ctx,
        apply_input("checkout", &approved.value.preview.fingerprint),
    )
    .unwrap();

    // `deliver` pushed; local and remote now hold the same commit. Previewing
    // again must not claim there is work waiting to be pushed.
    let report = deliver(&ctx, preview_input("checkout")).unwrap();

    let repo = &report.value.preview.repos[0];
    assert!(
        !repo
            .pending
            .iter()
            .any(|pending| pending.contains("not pushed")),
        "was: {:?}",
        repo.pending
    );
}

#[test]
fn a_failed_push_is_a_warning_and_does_not_block_the_others() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    approve_through_plan(&root);
    // Break web's remote before previewing, so the approved state says the
    // bogus URL — the fingerprint then matches when apply runs.
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let repos: Vec<Repo> = manifest
        .repos()
        .iter()
        .map(|repo| {
            if repo.name().as_str() == "web" {
                Repo::new(
                    RepoName::new("web").unwrap(),
                    root.join("no-such-origin").as_str(),
                    BranchName::new("main").unwrap(),
                )
            } else {
                repo.clone()
            }
        })
        .collect();
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        repos,
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    let ctx = Ctx::new(root.clone());

    let approved = deliver(&ctx, preview_input("checkout")).unwrap();
    let report = deliver(
        &ctx,
        apply_input("checkout", &approved.value.preview.fingerprint),
    )
    .unwrap();

    assert!(!report.is_clean(), "a failed push must not be a clean run");
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].subject, "web");
    assert_eq!(report.warnings[0].code, "deliver.push_failed");
    // Best-effort: api still landed.
    assert!(
        report
            .value
            .pushes
            .iter()
            .any(|push| push.repo.as_str() == "api" && push.ok)
    );
    assert!(
        report
            .value
            .pushes
            .iter()
            .any(|push| push.repo.as_str() == "web" && !push.ok)
    );
    assert!(remote_ref(&origin_of(&root, "api"), "checkout").is_some());
}

// -- ordering -------------------------------------------------------------

#[test]
fn push_preview_leaves_land_fields_absent() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let out = deliver(&ctx, preview_input("checkout")).expect("push preview");
    assert!(out.value.preview.repos[0].ff_possible.is_none());
    assert!(out.value.preview.repos[0].default_branch.is_none());
}

// -- per-repo outcome for a push that produced no PR ------------------------

#[test]
fn a_moved_base_reports_no_pull_request_on_the_repo_line() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let approved = deliver(&ctx, preview_input("checkout")).unwrap();

    // The base moves on the remote to a commit this bare clone never fetched.
    let origin = Utf8PathBuf::from(origin_of(&root, "api"));
    std::fs::write(origin.join("moved.md"), "moved\n").unwrap();
    git(&origin, &["add", "moved.md"]);
    git(&origin, &["commit", "-m", "move the base"]);

    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let feature_name = crate::domain::name::FeatureName::new("checkout").unwrap();
    let feature = super::read_feature(&layout, &feature_name).unwrap();
    let mut preview = approved.value.preview;
    preview.repos[0].action = DeliveryAction::NewPr;

    let report = crate::action::feature::deliver::push::execute(
        &crate::git::System,
        &layout,
        &manifest,
        &feature_name,
        &feature,
        preview,
    )
    .unwrap();

    let push = &report.value.pushes[0];
    assert!(push.ok, "the branch itself was pushed");
    assert!(push.pr.is_none());
    let detail = push.detail.as_deref().expect("a reason on the repo's line");
    assert!(
        detail.starts_with("no pull request:"),
        "the repo's own line must say no PR was made, was: {detail}"
    );
    assert!(
        push.fix.is_some(),
        "the repo's line must carry the way out of it"
    );

    let mut out = Vec::new();
    report.value.write_human(&mut out).unwrap();
    let rendered = String::from_utf8(out).unwrap();
    assert!(
        rendered.contains("api: pushed — no pull request:"),
        "was: {rendered}"
    );

    let json = serde_json::to_value(&report.value).unwrap();
    assert!(
        json["pushes"][0]["detail"]
            .as_str()
            .unwrap()
            .starts_with("no pull request:")
    );
    assert!(json["pushes"][0]["fix"].is_object());
}

#[test]
fn a_rejected_non_fast_forward_push_names_the_recovery_command() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let approved = deliver(&ctx, preview_input("checkout")).unwrap();
    deliver(
        &ctx,
        apply_input("checkout", &approved.value.preview.fingerprint),
    )
    .unwrap();

    // Rewriting the pushed tip is what `ivar feature rebase` does to it: the
    // remote now holds a commit this branch no longer contains.
    let layout = Layout::at(root.clone());
    let worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    git(&worktree, &["commit", "--amend", "-m", "work, rewritten"]);

    let approved = deliver(&ctx, preview_input("checkout")).unwrap();
    let report = deliver(
        &ctx,
        apply_input("checkout", &approved.value.preview.fingerprint),
    )
    .unwrap();

    let push = &report.value.pushes[0];
    assert!(!push.ok);
    let detail = push.detail.as_deref().unwrap();
    assert!(
        detail.contains("rejected"),
        "the rejection must be explained, was: {detail}"
    );
    let fix = push.fix.as_ref().expect("a recovery action");
    let command = fix.command.as_deref().expect("the exact command to run");
    assert!(command.contains("--force-with-lease"), "was: {command}");
    assert!(!fix.safe, "force-pushing stays a human's decision");

    let mut out = Vec::new();
    report.value.write_human(&mut out).unwrap();
    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("--force-with-lease"), "was: {rendered}");
}

#[test]
fn apply_with_only_pushes_only_the_selected_repo() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let only = vec!["web".to_owned()];
    let preview = deliver(
        &ctx,
        DeliverInput {
            only: only.clone(),
            ..preview_input("checkout")
        },
    )
    .unwrap();
    let fingerprint = preview.value.preview.fingerprint.clone();

    let report = deliver(
        &ctx,
        DeliverInput {
            only,
            ..apply_input("checkout", &fingerprint)
        },
    )
    .unwrap();

    assert_eq!(report.value.pushes.len(), 1);
    assert_eq!(report.value.pushes[0].repo.as_str(), "web");
    assert!(remote_ref(&origin_of(&root, "web"), "checkout").is_some());
    assert!(remote_ref(&origin_of(&root, "api"), "checkout").is_none());
}

#[test]
fn only_a_delivery_of_every_promotion_links_siblings() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let feature = read_feature(&layout, &FeatureName::new("checkout").unwrap()).unwrap();
    let preview_of = |only: &[&str]| {
        deliver(
            &ctx,
            DeliverInput {
                only: only.iter().map(|r| (*r).to_owned()).collect(),
                ..preview_input("checkout")
            },
        )
        .unwrap()
        .value
        .preview
    };

    assert!(!crate::action::feature::deliver::push::links_siblings(
        &preview_of(&["api"]),
        &feature
    ));
    assert!(crate::action::feature::deliver::push::links_siblings(
        &preview_of(&[]),
        &feature
    ));
    assert!(crate::action::feature::deliver::push::links_siblings(
        &preview_of(&["api", "web"]),
        &feature
    ));
}

#[test]
fn delivering_with_a_never_delivered_base_refuses_the_pr_with_a_deliver_parent_first_fix() {
    let (_guard, root) = hall_with_promoted_on(&["api"], Some("develop"), with_develop_base(false));
    approve_through_plan(&root);
    let fake = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&fake);
    as_github_remotes(&root);
    let origin = root.parent().unwrap().join("origins/api");

    // The base's branch is gone, and — unlike the merged-and-deleted case —
    // it was never merged into `main` first: nothing confirms it shipped.
    git(&origin, &["branch", "-D", "develop"]);

    let applied = deliver_json_expecting_warnings(&root, preview_input("checkout"));

    assert_eq!(applied["pushes"][0]["ok"], true);
    let warnings = applied["warnings"].as_array().expect("warnings array");
    assert!(
        warnings
            .iter()
            .any(|w| w["code"] == "feature.base_never_delivered"),
        "warnings were: {warnings:?}"
    );
    assert!(applied["preview"]["repos"][0]["pr_url"].is_null());
}

// `feature deliver` refuses to open or update a PR against a base that no
// longer supports it — merged and deleted, never delivered, moved on without
// a rebase, or simply unreachable. Each refusal is per repo (the push still
// lands) and never touches the network beyond what `remote_branch_tip`
// already reaches for.

#[test]
fn delivering_with_an_unreachable_remote_never_reports_the_base_absent() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let fake = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&fake);
    as_unreachable_github_remote(&root);

    let applied = deliver_json_expecting_warnings(&root, preview_input("checkout"));

    let warnings = applied["warnings"].as_array().expect("warnings array");
    assert!(
        warnings.iter().any(|w| w["code"] == "deliver.push_failed"),
        "the unanswered push is reported: {warnings:?}"
    );
    assert!(
        warnings.iter().all(|w| !w["what"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("absent")),
        "an unanswered remote must never be reported as an absent base: {warnings:?}"
    );
    assert!(applied["preview"]["repos"][0]["pr_url"].is_null());
    assert_eq!(
        fake.log().matches("pr create").count(),
        0,
        "no PR may be attempted for a repo the remote never received"
    );
}

fn advance_develop_on(origin: &Utf8Path) {
    git(origin, &["checkout", "develop"]);
    std::fs::write(origin.join("develop-later.txt"), "later\n").unwrap();
    git(origin, &["add", "develop-later.txt"]);
    git(origin, &["commit", "-m", "later develop work"]);
    git(origin, &["checkout", "main"]);
}

#[test]
fn delivering_with_a_base_that_moved_refuses_the_pr_but_still_pushes() {
    let (_guard, root) = hall_with_promoted_on(&["api"], Some("develop"), with_develop_base(false));
    approve_through_plan(&root);
    let fake = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&fake);
    as_github_remotes(&root);

    // Advance `develop` past what `checkout` was cut from, and pull that
    // straight into the bare clone's own `develop` ref — simulating that
    // ivar's local knowledge of the base has moved on, since `ivar sync`
    // itself only ever keeps the default branch's worktree current.
    let origin = root.parent().unwrap().join("origins/api");
    advance_develop_on(&origin);
    let bare = root.join(".ivar/repos/api/.bare");
    git(&bare, &["fetch", origin.as_str(), "develop:develop"]);

    let applied = deliver_json_expecting_warnings(&root, preview_input("checkout"));

    assert_eq!(
        applied["pushes"][0]["ok"], true,
        "pushing raw commits does not depend on the base"
    );
    let warnings = applied["warnings"].as_array().expect("warnings array");
    assert!(
        warnings.iter().any(|w| w["code"] == "feature.base_moved"),
        "warnings were: {warnings:?}"
    );
    assert!(applied["preview"]["repos"][0]["pr_url"].is_null());
    assert_eq!(fake.log().matches("pr create").count(), 0);
}

/// The bare clone's own `develop` ref is never re-fetched by anything this
/// test runs — `ivar sync` only ever keeps the default branch's worktree
/// current — so this is the ordinary case: the remote has moved on and
/// nothing local knows it yet. The check must ask the remote's own tip, not
/// trust a local ref that still (trivially) looks like an ancestor.
#[test]
fn delivering_with_a_base_that_moved_only_on_the_remote_still_refuses_the_pr() {
    let (_guard, root) = hall_with_promoted_on(&["api"], Some("develop"), with_develop_base(false));
    approve_through_plan(&root);
    let fake = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&fake);
    as_github_remotes(&root);

    advance_develop_on(&root.parent().unwrap().join("origins/api"));

    let applied = deliver_json_expecting_warnings(&root, preview_input("checkout"));

    assert_eq!(applied["pushes"][0]["ok"], true);
    let warnings = applied["warnings"].as_array().expect("warnings array");
    assert!(
        warnings.iter().any(|w| w["code"] == "feature.base_moved"),
        "a base advanced only on the remote must still refuse — warnings were: {warnings:?}"
    );
    assert!(applied["preview"]["repos"][0]["pr_url"].is_null());
    assert_eq!(fake.log().matches("pr create").count(), 0);
}

#[test]
fn delivering_with_a_merged_and_deleted_base_refuses_the_pr_with_a_rebase_onto_default_fix() {
    let (_guard, root) = hall_with_promoted_on(&["api"], Some("develop"), with_develop_base(true));
    approve_through_plan(&root);
    let fake = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&fake);
    as_github_remotes(&root);
    let origin = root.parent().unwrap().join("origins/api");

    // The base shipped and its branch was deleted — GitHub's usual
    // auto-delete-on-merge. ivar's bare clone keeps its own, now stale,
    // local `develop` ref, exactly as a developer's clone would.
    git(&origin, &["branch", "-D", "develop"]);

    let applied = deliver_json_expecting_warnings(&root, preview_input("checkout"));

    assert_eq!(applied["pushes"][0]["ok"], true);
    let warnings = applied["warnings"].as_array().expect("warnings array");
    let warning = warnings
        .iter()
        .find(|w| w["code"] == "feature.base_merged_and_deleted")
        .unwrap_or_else(|| panic!("warnings were: {warnings:?}"));
    assert!(warning["what"].as_str().unwrap().contains("develop"));
    assert!(applied["preview"]["repos"][0]["pr_url"].is_null());
}
