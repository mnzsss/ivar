use super::fixture::*;
use super::*;

#[test]
fn preview_lists_every_promoted_repo_with_its_delivery_facts() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());

    let report = deliver(&ctx, preview_input("checkout")).unwrap();

    assert!(report.is_clean());
    assert!(report.value.pushes.is_empty(), "preview must not push");
    assert_eq!(report.value.preview.repos.len(), 1);
    let repo = &report.value.preview.repos[0];
    assert_eq!(repo.repo.as_str(), "api");
    assert_eq!(repo.local_branch.as_str(), "checkout");
    assert!(repo.remote.contains("origins/api"), "was: {}", repo.remote);
    assert_eq!(repo.push_refspec, "checkout:refs/heads/checkout");
    assert_eq!(repo.action, DeliveryAction::PushOnly);
    assert_eq!(repo.base_branch.as_str(), "main");
    assert!(repo.dependencies.is_empty());
    // One commit beyond main, no upstream: the unpushed blocker.
    assert!(
        repo.blockers
            .iter()
            .any(|blocker| blocker.contains("1 commit(s) not pushed")),
        "was: {:?}",
        repo.blockers
    );
    // Preview is side-effect-free: the remote has no branch yet.
    assert!(remote_ref(&origin_of(&root, "api"), "checkout").is_none());
}

/// `base_branch` in the preview is the base `promote` actually recorded —
/// the feature's declared base, not always the repo's default branch.
#[test]
fn preview_shows_the_recorded_base_not_always_the_default_branch() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();

    let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
    git(&origin, &["checkout", "-b", "develop"]);
    std::fs::write(origin.join("develop-only.txt"), "develop\n").unwrap();
    git(&origin, &["add", "develop-only.txt"]);
    git(&origin, &["commit", "-m", "develop work"]);
    git(&origin, &["checkout", "main"]);

    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            origin.as_str(),
            BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: Some("develop".to_owned()),
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    let report = deliver(&ctx, preview_input("checkout")).unwrap();

    assert_eq!(
        report.value.preview.repos[0].base_branch.as_str(),
        "develop"
    );
}

#[test]
fn the_preview_has_a_stable_content_fingerprint() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());

    let first = deliver(&ctx, preview_input("checkout")).unwrap();
    let second = deliver(&ctx, preview_input("checkout")).unwrap();

    let fingerprint = &first.value.preview.fingerprint;
    assert_eq!(fingerprint.len(), 64, "a sha-256 hex digest");
    assert_eq!(fingerprint, &second.value.preview.fingerprint);
}

#[test]
fn a_feature_with_no_promoted_repos_previews_empty() {
    let (_guard, root) = hall_with_promoted(&[]);
    let ctx = Ctx::new(root.clone());

    let report = deliver(&ctx, preview_input("checkout")).unwrap();

    assert!(report.value.preview.repos.is_empty());
    assert_eq!(report.value.preview.fingerprint.len(), 64);
}

#[test]
fn a_dirty_worktree_is_listed_as_a_blocker() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    let worktree = Layout::at(root.clone()).repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(worktree.join("notes.md"), "mine\n").unwrap();

    let report = deliver(&ctx, preview_input("checkout")).unwrap();

    let repo = &report.value.preview.repos[0];
    assert!(
        repo.blockers
            .iter()
            .any(|blocker| blocker.contains("uncommitted changes")),
        "was: {:?}",
        repo.blockers
    );
}

#[test]
fn delivering_a_missing_feature_is_blocked() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root);

    let failure = deliver(&ctx, preview_input("ghost")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.not_found");
}

// -- apply: gating --------------------------------------------------------

#[test]
fn deliver_preview_reports_tree_blockers_and_apply_refuses_before_push() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);
    // An active leaf under the root blocks its delivery.
    child_of_checkout(&root, "child");
    let layout = Layout::at(root.clone());
    let mut leaf = crate::domain::feature::Feature::new(
        crate::domain::name::FeatureName::new("leaf").unwrap(),
        BranchName::new("leaf").unwrap(),
    );
    leaf.parent = Some(crate::domain::name::FeatureName::new("child").unwrap());
    leaf.write(&layout).unwrap();

    // The preview fingerprints the blockers and reports them.
    let report = deliver(&ctx, preview_input("checkout")).unwrap();
    assert_eq!(report.value.preview.tree_blockers.len(), 2);
    let names: Vec<&str> = report
        .value
        .preview
        .tree_blockers
        .iter()
        .map(|blocker| blocker.feature.as_str())
        .collect();
    assert_eq!(names, ["child", "leaf"]);
    assert_eq!(report.value.preview.tree_blockers[0].depth, 1);

    // Apply refuses before any push.
    let fingerprint = report.value.preview.fingerprint.clone();
    let failure = deliver(&ctx, apply_input("checkout", &fingerprint)).unwrap_err();
    assert_eq!(failure.code, "deliver.descendants_block");
    assert!(failure.actual.as_deref().unwrap().contains("child"));
}

#[test]
fn deliver_ignores_abandoned_descendants_but_sees_active_grandchildren() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);
    child_of_checkout(&root, "abandoned");
    let layout = Layout::at(&root);
    let mut grandchild = crate::domain::feature::Feature::new(
        crate::domain::name::FeatureName::new("active").unwrap(),
        BranchName::new("active").unwrap(),
    );
    grandchild.parent = Some(crate::domain::name::FeatureName::new("abandoned").unwrap());
    grandchild.write(&layout).unwrap();
    crate::action::feature::lifecycle::write_close(
        &layout,
        &crate::domain::name::FeatureName::new("abandoned").unwrap(),
        crate::domain::feature::PromotionOutcome::Abandoned,
    )
    .unwrap();

    let report = deliver(&ctx, preview_input("checkout")).unwrap();
    let names: Vec<&str> = report
        .value
        .preview
        .tree_blockers
        .iter()
        .map(|blocker| blocker.feature.as_str())
        .collect();
    assert_eq!(
        names,
        ["active"],
        "abandoned history does not block, but its active grandchild does"
    );
}

// -- rendering ------------------------------------------------------------

#[test]
fn the_human_preview_surface_lists_each_repo_and_the_fingerprint() {
    let outcome = DeliverOutcome {
        root: Utf8PathBuf::from("/hall"),
        preview: DeliveryPreview {
            feature: FeatureName::new("checkout").unwrap(),
            mode: DeliveryMode::Push,
            plan_gate: GateState::Approved,
            repos: vec![delivery_repo("api", vec![])],
            tree_blockers: Vec::new(),
            fingerprint: "abc123".to_owned(),
        },
        pushes: Vec::new(),
        land: Vec::new(),
        checks: Vec::new(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("Delivery preview for `checkout` in /hall:"));
    assert!(rendered.contains("branch:  checkout"));
    assert!(rendered.contains("refspec: checkout:refs/heads/checkout"));
    assert!(rendered.contains("base:    main"));
    assert!(rendered.contains("action:  push only"));
    assert!(rendered.contains("blockers: none"));
    assert!(rendered.contains("fingerprint: abc123"));
}

#[test]
fn preview_without_mode_defaults_to_push() {
    let json = serde_json::json!({
        "feature": "checkout",
        "plan_gate": "approved",
        "repos": [],
        "fingerprint": ""
    });
    let preview: DeliveryPreview = serde_json::from_value(json).expect("legacy preview");
    assert_eq!(preview.mode, DeliveryMode::Push);
}
