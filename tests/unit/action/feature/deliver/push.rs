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

/// The tip of `branch` on `origin`, without the ref name `ls-remote` adds.
fn remote_tip(origin: &str, branch: &str) -> Option<String> {
    remote_ref(origin, branch).map(|line| line.split_whitespace().next().unwrap().to_owned())
}

fn tip_of(worktree: &Utf8Path) -> String {
    git_stdout(worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_owned()
}

fn api_worktree(root: &Utf8Path) -> Utf8PathBuf {
    Layout::at(root.to_path_buf()).repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    )
}

/// A commit ivar never pushed, landing on the remote's `branch`.
fn push_stranger_commit(origin: &Utf8Path, branch: &str) {
    git(origin, &["checkout", branch]);
    std::fs::write(origin.join("stranger.md"), "stranger\n").unwrap();
    git(origin, &["add", "stranger.md"]);
    git(origin, &["commit", "-m", "somebody else's work"]);
    git(origin, &["checkout", "main"]);
}

#[test]
fn a_rejected_non_fast_forward_push_over_a_stranger_commit_names_the_recovery_command() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let approved = deliver(&ctx, preview_input("checkout")).unwrap();
    deliver(
        &ctx,
        apply_input("checkout", &approved.value.preview.fingerprint),
    )
    .unwrap();

    // Somebody else adds a commit to the pushed branch, and the branch is
    // rewritten as `ivar feature rebase` would: the remote now holds a commit
    // that no delivery of this feature pushed.
    let origin = root.parent().unwrap().join("origins/api");
    push_stranger_commit(&origin, "checkout");
    let stranger = remote_tip(origin.as_str(), "checkout").unwrap();
    git(
        &api_worktree(&root),
        &["commit", "--amend", "-m", "work, rewritten"],
    );

    let approved = deliver(&ctx, preview_input("checkout")).unwrap();
    assert!(
        !approved.value.preview.repos[0]
            .pending
            .iter()
            .any(|line| line.contains("--force-with-lease")),
        "a force-push over a commit nobody delivered must not be announced: {:?}",
        approved.value.preview.repos[0].pending
    );
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
    assert_eq!(fix.code, "deliver.force_with_lease");
    let command = fix.command.as_deref().expect("the exact command to run");
    assert!(command.contains("--force-with-lease"), "was: {command}");
    assert!(!fix.safe, "force-pushing stays a human's decision");
    assert_eq!(
        remote_tip(origin.as_str(), "checkout"),
        Some(stranger),
        "the stranger's commit must still be the remote tip"
    );

    let mut out = Vec::new();
    report.value.write_human(&mut out).unwrap();
    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("--force-with-lease"), "was: {rendered}");
}

#[test]
fn a_branch_rewritten_since_its_delivery_is_replaced_with_a_lease_over_the_delivered_tip() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let approved = deliver(&ctx, preview_input("checkout")).unwrap();
    deliver(
        &ctx,
        apply_input("checkout", &approved.value.preview.fingerprint),
    )
    .unwrap();
    let origin = origin_of(&root, "api");
    let delivered = remote_tip(&origin, "checkout").unwrap();

    // What `ivar feature rebase` does to the pushed tip.
    let worktree = api_worktree(&root);
    git(&worktree, &["commit", "--amend", "-m", "work, rewritten"]);
    let rewritten = tip_of(&worktree);
    assert_ne!(rewritten, delivered);

    let approved = deliver(&ctx, preview_input("checkout")).unwrap();
    let pending = &approved.value.preview.repos[0].pending;
    assert!(
        pending
            .iter()
            .any(|line| line.contains("rewritten") && line.contains(&delivered)),
        "the preview must announce the force-push over the delivered tip: {pending:?}"
    );
    let report = deliver(
        &ctx,
        apply_input("checkout", &approved.value.preview.fingerprint),
    )
    .unwrap();

    assert!(report.is_clean(), "{:?}", report.warnings);
    let push = &report.value.pushes[0];
    assert!(push.ok);
    assert!(push.fix.is_none());
    let detail = push.detail.as_deref().expect("the replacement is reported");
    assert!(
        detail.contains("force-pushed with lease") && detail.contains(&delivered),
        "was: {detail}"
    );
    assert_eq!(remote_tip(&origin, "checkout"), Some(rewritten.clone()));
    let feature = read_feature(
        &Layout::at(root.clone()),
        &FeatureName::new("checkout").unwrap(),
    )
    .unwrap();
    assert_eq!(
        feature.promotions[&RepoName::new("api").unwrap()]
            .delivered_sha
            .as_deref(),
        Some(rewritten.as_str()),
        "the replacement is itself what the next delivery recognises"
    );

    let mut out = Vec::new();
    report.value.write_human(&mut out).unwrap();
    let rendered = String::from_utf8(out).unwrap();
    assert!(
        rendered.contains("force-pushed with lease"),
        "was: {rendered}"
    );
    assert!(rendered.contains(&delivered), "was: {rendered}");
}

#[test]
fn a_rewritten_branch_is_replaced_and_its_pull_request_updated() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    let origin = root.parent().unwrap().join("origins/api");
    deliver_json(&root, preview_input("checkout"));
    let delivered = remote_tip(origin.as_str(), "checkout").unwrap();
    git(
        &api_worktree(&root),
        &["commit", "--amend", "-m", "work, rewritten"],
    );

    let applied = deliver_json(&root, preview_input("checkout"));

    assert_eq!(applied["pushes"][0]["ok"], true);
    assert!(
        applied["pushes"][0]["detail"]
            .as_str()
            .unwrap()
            .contains(&delivered)
    );
    assert!(
        !applied["pushes"][0]["pr"].is_null(),
        "the PR is reported after the replacement: {applied}"
    );
    assert_eq!(
        remote_tip(origin.as_str(), "checkout"),
        Some(tip_of(&api_worktree(&root)))
    );
}

#[test]
fn the_pushed_tip_is_recorded_even_when_no_pull_request_follows_and_survives_a_sync() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let feature_name = FeatureName::new("checkout").unwrap();
    let preview = deliver(&ctx, preview_input("checkout"))
        .unwrap()
        .value
        .preview;
    let feature = read_feature(&layout, &feature_name).unwrap();
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let mut preview = preview;
    // A PR is wanted, but the base is gone from the remote: the push lands
    // and the PR is refused.
    preview.repos[0].action = DeliveryAction::NewPr;
    preview.repos[0].base_branch = BranchName::new("gone").unwrap();

    let report = crate::action::feature::deliver::push::execute(
        &crate::git::System,
        &layout,
        &manifest,
        &feature_name,
        &feature,
        preview,
    )
    .unwrap();

    assert!(report.value.pushes[0].ok);
    assert!(report.value.pushes[0].pr.is_none());
    let pushed = tip_of(&api_worktree(&root));
    let recorded = |layout: &Layout| {
        read_feature(layout, &feature_name).unwrap().promotions[&RepoName::new("api").unwrap()]
            .delivered_sha
            .clone()
    };
    assert_eq!(recorded(&layout), Some(pushed.clone()));

    // A sync fetches over the bare's tracking refs; the record is not one.
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    assert_eq!(recorded(&layout), Some(pushed));
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

/// The blockers in `preview`, as `--preview --json` prints them.
fn blockers_of(preview: &serde_json::Value) -> Vec<String> {
    // Absent from the document when there are none.
    preview["blockers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|blocker| blocker.as_str().unwrap().to_owned())
        .collect()
}

/// Apply the delivery `preview` approved, and expect it refused by the
/// blockers it listed — with nothing pushed and no PR attempted.
fn assert_refused_before_any_push(
    root: &Utf8Path,
    origin: &Utf8Path,
    fake: &FakeGh,
    preview: &serde_json::Value,
) {
    let failure = deliver(
        &Ctx::new(root.to_path_buf()),
        apply_input(
            "checkout",
            preview["preview"]["fingerprint"].as_str().unwrap(),
        ),
    )
    .expect_err("a delivery with a base blocker must be refused");

    assert_eq!(failure.code, "deliver.blocked");
    assert!(
        remote_ref(origin.as_str(), "checkout").is_none(),
        "nothing may be pushed: the remote still has no `checkout`"
    );
    assert_eq!(fake.log().matches("pr create").count(), 0);
}

#[test]
fn a_never_delivered_base_blocks_the_preview_with_a_deliver_parent_first_fix() {
    let (_guard, root) = hall_with_promoted_on(&["api"], Some("develop"), with_develop_base(false));
    approve_through_plan(&root);
    let fake = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&fake);
    as_github_remotes(&root);
    let origin = root.parent().unwrap().join("origins/api");

    // The base's branch is gone, and — unlike the merged-and-deleted case —
    // it was never merged into `main` first: nothing confirms it shipped.
    git(&origin, &["branch", "-D", "develop"]);

    let preview = preview_json(&root, preview_input("checkout"));

    let blockers = blockers_of(&preview);
    assert!(
        blockers.iter().any(|blocker| blocker.starts_with("api: ")
            && blocker.contains("nothing confirms it was ever delivered")
            && blocker.contains("Deliver the feature that owns `develop` first")),
        "blockers were: {blockers:?}"
    );
    assert_refused_before_any_push(&root, &origin, &fake, &preview);
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

    // The remote did not answer, so the base is unconfirmed: a blocker of its
    // own, which the preview owes the human instead of promising an apply
    // that cannot work.
    let preview = preview_json(&root, preview_input("checkout"));

    let blockers = blockers_of(&preview);
    assert!(
        blockers
            .iter()
            .any(|blocker| blocker.contains("could not be confirmed")),
        "blockers were: {blockers:?}"
    );
    assert!(
        blockers
            .iter()
            .all(|blocker| !blocker.to_lowercase().contains("absent")),
        "an unanswered remote must never be reported as an absent base: {blockers:?}"
    );
    let failure = deliver(
        &Ctx::new(root.clone()),
        apply_input(
            "checkout",
            preview["preview"]["fingerprint"].as_str().unwrap(),
        ),
    )
    .expect_err("the unconfirmed base refuses the apply");
    assert_eq!(failure.code, "deliver.blocked");
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
fn a_base_that_moved_blocks_the_preview_with_a_rebase_fix_and_pushes_nothing() {
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

    let preview = preview_json(&root, preview_input("checkout"));

    let blockers = blockers_of(&preview);
    assert!(
        blockers.iter().any(|blocker| blocker.starts_with("api: ")
            && blocker.contains("no longer built on `develop`'s current tip")
            && blocker.contains("ivar feature rebase checkout --repo api")),
        "blockers were: {blockers:?}"
    );
    assert_refused_before_any_push(&root, &origin, &fake, &preview);
}

/// The bare clone's own `develop` ref is never re-fetched by anything this
/// test runs — `ivar sync` only ever keeps the default branch's worktree
/// current — so this is the ordinary case: the remote has moved on and
/// nothing local knows it yet. The check must ask the remote's own tip, not
/// trust a local ref that still (trivially) looks like an ancestor.
#[test]
fn a_base_that_moved_only_on_the_remote_still_blocks_the_preview() {
    let (_guard, root) = hall_with_promoted_on(&["api"], Some("develop"), with_develop_base(false));
    approve_through_plan(&root);
    let fake = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&fake);
    as_github_remotes(&root);
    let origin = root.parent().unwrap().join("origins/api");

    advance_develop_on(&origin);

    let preview = preview_json(&root, preview_input("checkout"));

    let blockers = blockers_of(&preview);
    assert!(
        blockers
            .iter()
            .any(|blocker| blocker.contains("ivar feature rebase checkout --repo api")),
        "a base advanced only on the remote must still block — blockers were: {blockers:?}"
    );
    assert_refused_before_any_push(&root, &origin, &fake, &preview);
}

#[test]
fn a_base_that_moves_after_the_preview_is_a_fingerprint_mismatch_not_a_push() {
    let (_guard, root) = hall_with_promoted_on(&["api"], Some("develop"), with_develop_base(false));
    approve_through_plan(&root);
    let fake = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&fake);
    as_github_remotes(&root);
    let origin = root.parent().unwrap().join("origins/api");
    let preview = preview_json(&root, preview_input("checkout"));
    assert!(blockers_of(&preview).is_empty(), "{preview}");

    advance_develop_on(&origin);

    let failure = deliver(
        &Ctx::new(root.clone()),
        apply_input(
            "checkout",
            preview["preview"]["fingerprint"].as_str().unwrap(),
        ),
    )
    .expect_err("the approved state no longer holds");
    assert_eq!(failure.code, "deliver.fingerprint_mismatch");
    assert!(remote_ref(origin.as_str(), "checkout").is_none());
}

#[test]
fn a_merged_and_deleted_base_blocks_the_preview_with_a_rebase_onto_default_fix() {
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

    let preview = preview_json(&root, preview_input("checkout"));

    let blockers = blockers_of(&preview);
    let blocker = blockers
        .iter()
        .find(|blocker| blocker.contains("looks merged into `main`"))
        .unwrap_or_else(|| panic!("blockers were: {blockers:?}"));
    assert!(blocker.contains("develop"));
    assert!(blocker.contains("ivar feature rebase <feature> --onto main"));
    assert_refused_before_any_push(&root, &origin, &fake, &preview);
}

#[test]
fn a_repo_that_only_pushes_is_not_blocked_by_its_base() {
    // Non-GitHub remotes have no PR to draw a diff for, so no base to check.
    let (_guard, root) = hall_with_promoted_on(&["api"], Some("develop"), with_develop_base(false));
    approve_through_plan(&root);
    let origin = root.parent().unwrap().join("origins/api");
    git(&origin, &["branch", "-D", "develop"]);

    let preview = preview_json(&root, preview_input("checkout"));

    assert!(blockers_of(&preview).is_empty(), "{preview}");
}
