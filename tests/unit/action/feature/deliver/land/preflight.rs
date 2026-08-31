use super::super::fixture::*;
use super::super::*;

#[test]
fn diverged_default_is_not_fast_forwardable() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    std::fs::write(default_worktree.join("main.txt"), "main commit\n").unwrap();
    git(&default_worktree, &["add", "main.txt"]);
    git(&default_worktree, &["commit", "-m", "main commit"]);

    let preview = deliver(&ctx, land_preview_input("checkout"))
        .expect("land preview")
        .value
        .preview;
    assert_eq!(preview.repos[0].ff_possible, Some(false));
    assert!(
        preview.repos[0]
            .blockers
            .iter()
            .any(|b| b.contains("cannot fast-forward"))
    );

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("non-ff must block");
    assert_eq!(failure.code, "deliver.land_not_fast_forward");
    let fix = failure
        .fix_actions
        .first()
        .expect("a blocked land must say how to unblock");
    assert_eq!(
        fix.command.as_deref().unwrap(),
        "ivar feature rebase checkout"
    );
}

#[test]
fn dirty_default_worktree_blocks_and_is_left_untouched() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    std::fs::write(default_worktree.join("dirty.txt"), "uncommitted changes\n").unwrap();

    let before = std::fs::read(default_worktree.join("dirty.txt")).unwrap();
    let preview = deliver(&ctx, land_preview_input("checkout"))
        .expect("land preview")
        .value
        .preview;
    assert!(
        preview.repos[0]
            .blockers
            .iter()
            .any(|b| b.contains("uncommitted changes"))
    );

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("dirty must block");
    assert_eq!(failure.code, "deliver.land_dirty_worktree");
    let after = std::fs::read(default_worktree.join("dirty.txt")).unwrap();
    assert_eq!(before, after);
}

#[test]
fn rebase_in_progress_blocks() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let worktree_git_dir = crate::git::read::worktree_git_dir(&default_worktree).unwrap();
    std::fs::create_dir_all(worktree_git_dir.join("rebase-merge")).unwrap();

    let preview = deliver(&ctx, land_preview_input("checkout"))
        .expect("land preview")
        .value
        .preview;
    assert!(
        preview.repos[0]
            .blockers
            .iter()
            .any(|b| b.contains("rebase is in progress"))
    );

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("rebase in progress must block");
    assert_eq!(failure.code, "deliver.land_rebase_in_progress");
}

#[test]
fn land_no_repos_blocks() {
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

    create_action(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    approve_through_plan(&root);

    let preview = deliver(&ctx, land_preview_input("checkout"))
        .expect("land preview")
        .value
        .preview;
    assert!(preview.repos.is_empty());

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("no repos must block");
    assert_eq!(failure.code, "deliver.land_no_repos");
}

#[test]
fn one_blocked_repo_blocks_the_whole_land_and_writes_nothing() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let default_worktree_web = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    git_stdout(
        &default_worktree_web,
        &["commit", "--allow-empty", "-m", "diverge web main"],
    );

    let feature_worktree_api = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_worktree_api.join("api_change.txt"), "api change\n").unwrap();
    git_stdout(&feature_worktree_api, &["add", "api_change.txt"]);
    git_stdout(
        &feature_worktree_api,
        &["commit", "-m", "api feature commit"],
    );

    let before = snapshot_all_worktrees(&root);

    let preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
            ..Default::default()
        },
    )
    .expect("preview")
    .value
    .preview;

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("a blocked repo must block the batch");
    assert_eq!(failure.code, "deliver.land_not_fast_forward");

    let after = snapshot_all_worktrees(&root);
    assert_eq!(before, after, "no repo may be written when land is blocked");
}

#[test]
fn land_runs_verification_gate_and_refuses_on_failure() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let manifest = read_manifest(&layout).unwrap();
    let repos: Vec<_> = manifest
        .repos()
        .iter()
        .map(|r| {
            if r.name().as_str() == "api" {
                r.clone().with_checks(vec!["exit 1".to_owned()])
            } else {
                r.clone()
            }
        })
        .collect();
    let new_manifest = Manifest::new(
        manifest.name().clone(),
        manifest.providers().clone(),
        repos,
        None,
    )
    .unwrap();
    Manifest::write(&layout, &new_manifest).unwrap();

    let preview = deliver(&ctx, land_preview_input("checkout"))
        .expect("land preview")
        .value
        .preview;

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(preview.fingerprint),
            ..Default::default()
        },
    )
    .expect_err("failing verification checks must refuse land");

    assert_eq!(failure.code, "deliver.checks_failed");
    assert!(
        failure
            .what
            .contains("verification checks failed for repo `api`")
    );
}

#[test]
fn land_undeclared_default_branch_returns_declare_default_branch_error() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let feature = read_feature(&layout, &FeatureName::new("checkout").unwrap()).unwrap();

    let push_preview = deliver(&ctx, preview_input("checkout"))
        .expect("push preview")
        .value
        .preview;

    assert!(push_preview.repos[0].default_branch.is_none());

    let failure = crate::action::feature::deliver::land::preflight(
        &crate::git::System,
        &layout,
        &feature,
        &push_preview,
    )
    .expect_err("undeclared default branch must return Err");

    assert_eq!(failure.code, "deliver.declare_default_branch");
    assert_eq!(
        failure.fix_actions[0].code,
        "deliver.declare_default_branch"
    );
}
