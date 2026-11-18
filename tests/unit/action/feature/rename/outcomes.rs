use super::*;

#[test]
fn rename_refuses_when_name_and_branch_unchanged() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    create_action(
        &ctx,
        CreateInput {
            name: "my-feat".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let err = rename(&ctx, rename_input("my-feat", Some("my-feat"), None)).unwrap_err();
    assert_eq!(err.status, Status::Blocked);
    assert_eq!(err.code, "feature.rename_noop");
}

#[test]
fn rename_feature_name_success() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    create_action(
        &ctx,
        CreateInput {
            name: "old-feat".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let report = rename(&ctx, rename_input("old-feat", Some("new-feat"), None)).unwrap();
    assert!(report.is_clean());
    let outcome = report.value;
    assert_eq!(outcome.old_name.as_str(), "old-feat");
    assert_eq!(outcome.new_name.as_str(), "new-feat");
}

#[test]
fn rename_record_writing_and_preservation() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "record-old".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let old_name = FeatureName::new("record-old").unwrap();
    let new_name = FeatureName::new("record-new").unwrap();
    let new_branch = BranchName::new("feat/record-new-branch").unwrap();

    let report = rename(
        &ctx,
        rename_input(
            "record-old",
            Some("record-new"),
            Some("feat/record-new-branch"),
        ),
    )
    .unwrap();
    assert!(report.is_clean());

    assert!(Feature::read(&layout, &old_name).unwrap().is_none());

    let updated = Feature::read(&layout, &new_name).unwrap().unwrap();
    assert_eq!(updated.name, new_name);
    assert_eq!(updated.branch, new_branch);
}

#[test]
fn rename_updates_child_parent_reference() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "parent-old".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "child-feat".to_owned(),
            branch: None,
            base: None,
            parent: Some("parent-old".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let child_name = FeatureName::new("child-feat").unwrap();
    let child_before = Feature::read(&layout, &child_name).unwrap().unwrap();
    assert_eq!(child_before.parent.as_ref().unwrap().as_str(), "parent-old");

    rename(&ctx, rename_input("parent-old", Some("parent-new"), None)).unwrap();

    let child_after = Feature::read(&layout, &child_name).unwrap().unwrap();
    assert_eq!(child_after.parent.as_ref().unwrap().as_str(), "parent-new");
}

#[test]
fn rename_updates_and_rematerialises_session() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "sess-old".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let session_report = crate::action::session::start::start(
        &ctx,
        crate::action::session::start::StartInput {
            feature: Some("sess-old".to_owned()),
            resume: false,
            provider: None,
            detached: true,
            relay: false,
            prompt: None,
        },
    )
    .unwrap();

    let _outcome = session_report.value;
    let old_name = FeatureName::new("sess-old").unwrap();
    let new_name = FeatureName::new("sess-new").unwrap();

    let initial_sessions = lookup::list_feature(&layout, &old_name).unwrap();
    assert_eq!(initial_sessions.len(), 1);

    rename(&ctx, rename_input("sess-old", Some("sess-new"), None)).unwrap();

    let updated_sessions = lookup::list_feature(&layout, &new_name).unwrap();
    assert_eq!(updated_sessions.len(), 1);
    let s_ref = &updated_sessions[0];
    assert_eq!(s_ref.state.as_ref().unwrap().feature, Some(new_name));
    assert!(fs::is_dir(&s_ref.view_dir).unwrap());
}

#[test]
fn find_transition_discovers_marker_in_new_path() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "trans-old".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let old_name = FeatureName::new("trans-old").unwrap();
    let new_name = FeatureName::new("trans-new").unwrap();

    let source = Feature::read(&layout, &old_name).unwrap().unwrap();
    let plan = plan::build(
        &layout,
        &read_manifest(&layout).unwrap(),
        &git::System,
        &source,
        new_name.clone(),
        None,
    )
    .unwrap()
    .0;

    let new_dir = layout.feature_dir(&new_name);
    fs::ensure_dir(&new_dir).unwrap();
    let marker_path = new_dir.join(".renaming");
    let transition = steps::Transition {
        version: 1,
        plan,
        direction: steps::Direction::Forward,
        step: steps::Step::MoveFeatureDir,
    };
    json::write_canonical(&marker_path, &transition).unwrap();

    let found_old = steps::find_transition(&layout, &old_name).unwrap();
    assert!(found_old.is_some());
    let (found_path, found_t) = found_old.unwrap();
    assert_eq!(found_path, marker_path);
    assert_eq!(found_t.plan.new_name, new_name);

    let found_new = steps::find_transition(&layout, &new_name).unwrap();
    assert!(found_new.is_some());
    assert_eq!(found_new.unwrap().0, marker_path);
}

#[test]
fn rename_name_only_and_branch_only_behaviour() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "feat-a".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    // Name-only rename
    let outcome = rename(&ctx, rename_input("feat-a", Some("feat-b"), None))
        .unwrap()
        .value;
    assert_eq!(outcome.old_name.as_str(), "feat-a");
    assert_eq!(outcome.new_name.as_str(), "feat-b");
    assert_eq!(outcome.old_branch, outcome.new_branch);
    assert!(fs::is_dir(&layout.feature_dir(&FeatureName::new("feat-b").unwrap())).unwrap());
    assert!(!fs::is_dir(&layout.feature_dir(&FeatureName::new("feat-a").unwrap())).unwrap());

    // Branch-only rename
    let outcome2 = rename(
        &ctx,
        rename_input("feat-b", None, Some("feat/new-branch-name")),
    )
    .unwrap()
    .value;
    assert_eq!(outcome2.old_name.as_str(), "feat-b");
    assert_eq!(outcome2.new_name.as_str(), "feat-b");
    assert_ne!(outcome2.old_branch, outcome2.new_branch);
    assert_eq!(outcome2.new_branch.as_str(), "feat/new-branch-name");
    assert!(fs::is_dir(&layout.feature_dir(&FeatureName::new("feat-b").unwrap())).unwrap());
}

#[test]
fn rename_remote_rollback() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();
    let manifest = read_manifest(&layout).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "remote-feat".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let old_name = FeatureName::new("remote-feat").unwrap();
    let source = Feature::read(&layout, &old_name).unwrap().unwrap();
    let mut plan = plan::build(
        &layout,
        &manifest,
        &git::System,
        &source,
        old_name.clone(),
        Some(&BranchName::new("new-branch").unwrap()),
    )
    .unwrap()
    .0;

    use crate::domain::name::RepoName;
    plan.repos.push(super::plan::RepoRenamePlan {
        repo: RepoName::new("acme").unwrap(),
        old_worktree: layout.repo_worktree(&RepoName::new("acme").unwrap(), &source.branch),
        new_worktree: layout.repo_worktree(
            &RepoName::new("acme").unwrap(),
            &BranchName::new("new-branch").unwrap(),
        ),
        old_branch: source.branch.clone(),
        new_branch: BranchName::new("new-branch").unwrap(),
        old_remote: Some("origin".to_owned()),
        old_remote_tip: Some("old-tip-sha".to_owned()),
    });

    let dummy_git = DummyRemoteGit::default();
    let res = steps::undo_step(
        &layout,
        &manifest,
        &dummy_git,
        &plan,
        steps::Step::RemoteOps,
    );
    assert_eq!(res.unwrap(), steps::Step::MoveWorktrees);

    let pushed = dummy_git.pushed.lock().unwrap();
    assert_eq!(pushed.len(), 1);
    assert_eq!(pushed[0].1, "origin");
    assert_eq!(pushed[0].2, "old-tip-sha");
    assert_eq!(pushed[0].3, format!("refs/heads/{}", source.branch));

    let deleted = dummy_git.deleted.lock().unwrap();
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].1, "origin");
    assert_eq!(deleted[0].2, "new-branch");
    assert_eq!(deleted[0].3, "new-tip-sha");
}
