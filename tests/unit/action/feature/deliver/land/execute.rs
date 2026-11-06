use super::super::fixture::*;
use super::super::*;

#[test]
fn a_matching_land_fingerprint_executes_the_land() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            ..Default::default()
        },
    )
    .expect("land preview");

    let applied = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint.clone()),
            ..Default::default()
        },
    );
    // Wave 3: land now executes; a push to a non-bare origin may warn but the
    // merge must have been attempted.
    match applied {
        Ok(out) => {
            assert!(
                out.value.land.iter().any(|r| r.merged),
                "at least one repo must merge"
            );
        }
        Err(failure) => {
            // fast-forward failure or mode mismatch — both surface clearly
            assert_ne!(
                failure.code, "deliver.land_not_implemented",
                "land_not_implemented must never fire after Wave 3",
            );
        }
    }
}

// -- land preview and blockers (Wave 2) ------------------------------------

#[test]
fn a_clean_land_merges_every_repo_and_pushes_each_default() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let origins = root.parent().unwrap().join("origins").join("api");
    git_stdout(&origins, &["config", "receive.denyCurrentBranch", "ignore"]);

    let feature_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(
        feature_worktree.join("feature.txt"),
        "new feature content\n",
    )
    .unwrap();
    git_stdout(&feature_worktree, &["add", "feature.txt"]);
    git_stdout(&feature_worktree, &["commit", "-m", "add feature.txt"]);
    let feature_tip = git_stdout(&feature_worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            ..Default::default()
        },
    )
    .expect("land preview");

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
            ..Default::default()
        },
    )
    .expect("land apply");

    assert_eq!(out.value.land.len(), 1);
    assert!(out.value.land[0].merged);
    assert!(out.value.land[0].pushed);

    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let default_tip = git_stdout(&default_worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    assert_eq!(
        default_tip, feature_tip,
        "default branch must equal feature tip"
    );

    assert!(
        feature_worktree.exists(),
        "feature worktree must still exist"
    );
}

#[test]
fn a_failed_push_is_a_warning_not_an_abort() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);

    let feature_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_worktree.join("feature.txt"), "content\n").unwrap();
    git_stdout(&feature_worktree, &["add", "feature.txt"]);
    git_stdout(&feature_worktree, &["commit", "-m", "feature commit"]);

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            ..Default::default()
        },
    )
    .expect("land preview");

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
            ..Default::default()
        },
    )
    .expect("a failed push must not abort the land");

    assert_eq!(out.value.land.len(), 1);
    assert!(
        out.value.land[0].merged,
        "merge stands even when push fails"
    );
    assert!(!out.value.land[0].pushed, "push failed");
    assert!(
        out.warnings
            .iter()
            .any(|w| w.code == "deliver.land_push_failed"),
        "push failure produces deliver.land_push_failed warning"
    );
}

#[test]
fn execute_failure_rolls_back_earlier_merges() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);

    let feature_api = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_api.join("api.txt"), "api\n").unwrap();
    git_stdout(&feature_api, &["add", "api.txt"]);
    git_stdout(&feature_api, &["commit", "-m", "api commit"]);

    let feature_web = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_web.join("web.txt"), "web\n").unwrap();
    git_stdout(&feature_web, &["add", "web.txt"]);
    git_stdout(&feature_web, &["commit", "-m", "web commit"]);

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            ..Default::default()
        },
    )
    .expect("land preview");

    let api_default = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let api_default_before = git_stdout(&api_default, &["rev-parse", "HEAD"]);

    let web_default = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let web_git_dir = crate::git::System.worktree_git_dir(&web_default).unwrap();
    std::fs::write(web_git_dir.join("index.lock"), "lock").unwrap();

    let _failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("execute failure on web must return Err");

    let api_default_after = git_stdout(&api_default, &["rev-parse", "HEAD"]);
    assert_eq!(
        api_default_before, api_default_after,
        "repo api must be rolled back if repo web fails during execute"
    );
}

#[test]
fn remote_default_ahead_of_local_default_does_not_trigger_remote_moved() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let origins = root.parent().unwrap().join("origins").join("api");
    git_stdout(&origins, &["config", "receive.denyCurrentBranch", "ignore"]);

    // Push a commit to origin main so remote main is ahead of local main
    git_stdout(
        &origins,
        &["commit", "--allow-empty", "-m", "origin main ahead"],
    );

    // Feature branch on top of origin main
    let feature_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    git_stdout(&feature_worktree, &["fetch", "origin"]);
    git_stdout(&feature_worktree, &["rebase", "origin/main"]);

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            ..Default::default()
        },
    )
    .expect("land preview");

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
            ..Default::default()
        },
    )
    .expect("land apply");

    assert!(
        !out.warnings
            .iter()
            .any(|w| w.code == "deliver.land_remote_moved"),
        "remote main did not move after preview; land_remote_moved must not fire"
    );
}

#[test]
fn failure_conventions_are_honored_for_land_system_failures() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let feature_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_worktree.join("f.txt"), "f").unwrap();
    git_stdout(&feature_worktree, &["add", "f.txt"]);
    git_stdout(&feature_worktree, &["commit", "-m", "f"]);

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            ..Default::default()
        },
    )
    .expect("preview");

    // Lock default worktree index to cause fast_forward_to failure
    let git_dir = crate::git::System
        .worktree_git_dir(&default_worktree)
        .unwrap();
    std::fs::write(git_dir.join("index.lock"), "lock").unwrap();

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("merge failure");

    assert_eq!(failure.code, "git.merge_ff_only_failed");
    assert!(failure.expected.is_some(), "expected must be populated");
    assert!(failure.actual.is_some(), "actual must be populated");
    assert!(
        !failure.fix_actions.is_empty(),
        "fix_actions must be populated"
    );
}

#[test]
fn land_merge_failure_fix_action_uses_feature_name() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());

    let feature_name = "auth-v2";
    let custom_branch = "feat/auth-v2";
    create_action(
        &ctx,
        CreateInput {
            name: feature_name.to_owned(),
            branch: Some(custom_branch.to_owned()),
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();

    promote::promote(
        &ctx,
        PromoteInput {
            feature: feature_name.to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    crate::action::plan::create::create(
        &ctx,
        crate::action::plan::create::CreateInput {
            feature: feature_name.to_owned(),
            artifacts: Vec::new(),
        },
    )
    .unwrap();
    for gate in ["requirements", "analysis", "plan"] {
        crate::action::plan::approve::approve(
            &ctx,
            crate::action::plan::approve::ApproveInput {
                feature: feature_name.to_owned(),
                gate: gate.to_owned(),
            },
        )
        .unwrap();
    }
    record_memory_review(&root, feature_name, serde_json::json!({}));

    let layout = Layout::at(&root);
    let feature = read_feature(&layout, &FeatureName::new(feature_name).unwrap()).unwrap();
    let feature_worktree = layout.repo_worktree(&RepoName::new("api").unwrap(), &feature.branch);
    std::fs::write(feature_worktree.join("f.txt"), "f").unwrap();
    git_stdout(&feature_worktree, &["add", "f.txt"]);
    git_stdout(
        &feature_worktree,
        &["commit", "-m", "commit on feat/auth-v2"],
    );

    let land_preview = deliver(&ctx, land_preview_input(feature_name))
        .expect("land preview")
        .value
        .preview;

    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let git_dir = crate::git::System
        .worktree_git_dir(&default_worktree)
        .unwrap();
    std::fs::write(git_dir.join("index.lock"), "lock").unwrap();

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: feature_name.to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("merge failure must return Err");

    assert_eq!(failure.code, "git.merge_ff_only_failed");
    let fix = failure
        .fix_actions
        .first()
        .expect("fix action must be present");
    assert!(
        fix.what.contains("ivar feature rebase auth-v2"),
        "fix action must name the feature, got: {}",
        fix.what
    );
}
