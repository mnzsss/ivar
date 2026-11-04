use super::*;

#[test]
fn previews_one_promoted_repo_without_writes() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let layout = Layout::at(root.clone());
    let bare = layout.repo_bare(&RepoName::new("api").unwrap());
    let git_sys = git::System;
    let before = git_sys.revision_commit(&bare, "checkout").unwrap();

    let preview = run_preview(&root);

    assert_eq!(preview.repos.len(), 1);
    assert!(preview.repos[0].is_delivered);
    assert!(preview.blockers.is_empty());
    let after = git_sys.revision_commit(&bare, "checkout").unwrap();
    assert_eq!(before, after);
}

#[test]
fn previews_repositories_in_manifest_order_and_branch_with_slash() {
    let (_guard, root) = hall_with_feature(&["web", "api"], Some("feat/checkout"));

    let preview = run_preview(&root);

    assert_eq!(preview.branch.as_str(), "feat/checkout");
    assert_eq!(
        preview
            .repos
            .iter()
            .map(|repo| repo.repo.as_str())
            .collect::<Vec<_>>(),
        vec!["api", "web"]
    );
}

#[test]
fn marks_empty_feature_explicitly() {
    let (_guard, root) = hall_with_feature(&[], None);

    let preview = run_preview(&root);

    assert!(preview.repos.is_empty());
    assert_eq!(preview.blockers, vec![CleanupBlocker::EmptyFeature]);
}

#[test]
fn reports_live_session_dirty_and_missing_worktree() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let session = SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap();
    fs::ensure_dir(&layout.feature_session(&feature, &session)).unwrap();
    let worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(worktree.join("dirty.txt"), "dirty").unwrap();

    let preview = run_preview(&root);

    assert!(
        preview
            .blockers
            .iter()
            .any(|blocker| matches!(blocker, CleanupBlocker::LiveSessions { .. }))
    );
    assert!(
        preview
            .blockers
            .iter()
            .any(|blocker| matches!(blocker, CleanupBlocker::DirtyWorktree { .. }))
    );
    remove_dir_all(&worktree).unwrap();
    let preview = run_preview(&root);
    assert!(
        preview
            .blockers
            .iter()
            .any(|blocker| matches!(blocker, CleanupBlocker::MissingWorktree { .. }))
    );
}

#[test]
fn reports_missing_clone() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let layout = Layout::at(root.clone());
    let repo = RepoName::new("api").unwrap();
    remove_dir_all(layout.repo_bare(&repo)).unwrap();
    let preview = run_preview(&root);
    assert!(
        preview
            .blockers
            .iter()
            .any(|blocker| matches!(blocker, CleanupBlocker::MissingClone { .. }))
    );
}

#[test]
fn preview_paths_to_remove_lists_worktrees_in_manifest_order_then_feature_dir() {
    let (_guard, root) = hall_with_feature(&["web", "api"], Some("feat/checkout"));
    let layout = Layout::at(root.clone());
    let feature_name = FeatureName::new("checkout").unwrap();
    let branch_name = BranchName::new("feat/checkout").unwrap();

    let preview = run_preview(&root);

    assert_eq!(
        preview.paths_to_remove,
        vec![
            layout.repo_worktree(&RepoName::new("api").unwrap(), &branch_name),
            layout.repo_worktree(&RepoName::new("web").unwrap(), &branch_name),
            layout.feature_dir(&feature_name),
        ]
    );
}

#[test]
fn empty_feature_preview_lists_feature_dir_only() {
    let (_guard, root) = hall_with_feature(&[], None);
    let layout = Layout::at(root.clone());
    let feature_name = FeatureName::new("checkout").unwrap();

    let preview = run_preview(&root);

    assert_eq!(
        preview.paths_to_remove,
        vec![layout.feature_dir(&feature_name)]
    );
}

#[test]
fn paths_and_fingerprint_are_deterministic_and_fingerprint_changes_with_promoted_repos() {
    let (_guard, root) = hall_with_feature(&["api", "web"], None);
    let ctx = Ctx::new(root.clone());

    create_feature(
        &ctx,
        CreateInput {
            name: "feat2".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    promote(
        &ctx,
        PromoteInput {
            feature: "feat2".to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    let outcome1 = cleanup(
        &ctx,
        CleanupInput {
            feature: "feat2".to_owned(),
            preview: true,
            record: None,
            session_id: None,
        },
    )
    .unwrap()
    .value;
    let preview1 = outcome1.preview;

    let outcome2 = cleanup(
        &ctx,
        CleanupInput {
            feature: "feat2".to_owned(),
            preview: true,
            record: None,
            session_id: None,
        },
    )
    .unwrap()
    .value;
    let preview2 = outcome2.preview;

    assert_eq!(preview1.paths_to_remove, preview2.paths_to_remove);
    assert_eq!(preview1.fingerprint, preview2.fingerprint);

    promote(
        &ctx,
        PromoteInput {
            feature: "feat2".to_owned(),
            repo: "web".to_owned(),
            base: None,
        },
    )
    .unwrap();

    let outcome3 = cleanup(
        &ctx,
        CleanupInput {
            feature: "feat2".to_owned(),
            preview: true,
            record: None,
            session_id: None,
        },
    )
    .unwrap()
    .value;
    let preview3 = outcome3.preview;

    assert_ne!(preview1.paths_to_remove, preview3.paths_to_remove);
    assert_ne!(preview1.fingerprint, preview3.fingerprint);
}

#[test]
fn write_human_includes_paths_to_remove_heading_and_list() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let ctx = Ctx::new(root.clone());
    let outcome = cleanup(
        &ctx,
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: true,
            record: None,
            session_id: None,
        },
    )
    .unwrap()
    .value;

    let mut buf = Vec::new();
    outcome.write_human(&mut buf).unwrap();
    let human = String::from_utf8(buf).unwrap();

    assert!(human.contains("Paths to remove:"));
    let feature_dir_str = ".ivar/features/checkout";
    assert!(human.contains(feature_dir_str));
    assert_eq!(
        human.matches(feature_dir_str).count(),
        1,
        "feature dir must appear exactly once in paths to remove"
    );
}

#[test]
fn preview_does_not_block_on_the_session_running_cleanup() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let own = SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap();
    fs::ensure_dir(&layout.feature_session(&feature, &own)).unwrap();

    let preview = preview_from_session(&root, &own);

    assert!(
        !preview
            .blockers
            .iter()
            .any(|blocker| matches!(blocker, CleanupBlocker::LiveSessions { .. })),
        "own session must not block: {:?}",
        preview.blockers
    );
}

#[test]
fn preview_still_blocks_on_other_live_sessions() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let own = SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap();
    let other = SessionId::new("7d1f0a2b-3c4d-4e5f-8a9b-0c1d2e3f4a5b").unwrap();
    fs::ensure_dir(&layout.feature_session(&feature, &own)).unwrap();
    fs::ensure_dir(&layout.feature_session(&feature, &other)).unwrap();

    let preview = preview_from_session(&root, &own);

    assert!(
        preview.blockers.contains(&CleanupBlocker::LiveSessions {
            sessions: vec![other],
        }),
        "only the other session blocks: {:?}",
        preview.blockers
    );
}
