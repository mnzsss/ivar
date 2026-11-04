use super::*;

#[test]
fn delete_refuse_rejects_parent_with_descendants_unchanged() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    create_action(
        &ctx,
        CreateInput {
            name: "child".to_owned(),
            branch: None,
            base: None,
            parent: Some("checkout".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let input = DeleteInput {
        name: "checkout".to_owned(),
        force: false,
        descendants: Descendants::Refuse,
    };
    let failure = delete(&ctx, input).unwrap_err();
    assert_eq!(failure.code, "feature.has_descendants");
}

#[test]
fn delete_ask_non_interactive_refuses_with_exact_yes_command() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone()); // default NonInteractive
    create_action(
        &ctx,
        CreateInput {
            name: "child".to_owned(),
            branch: None,
            base: None,
            parent: Some("checkout".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let input = DeleteInput {
        name: "checkout".to_owned(),
        force: false,
        descendants: Descendants::Ask,
    };
    let failure = delete(&ctx, input).unwrap_err();
    assert_eq!(failure.code, "feature.delete_subtree_needs_consent");
    let fix_cmd = failure.fix_actions[0]
        .command
        .as_deref()
        .expect("fix command present");
    assert_eq!(fix_cmd, "ivar feature delete checkout --yes");

    // With force: command carries --force
    let input_forced = DeleteInput {
        name: "checkout".to_owned(),
        force: true,
        descendants: Descendants::Ask,
    };
    let failure_forced = delete(&ctx, input_forced).unwrap_err();
    let fix_cmd_forced = failure_forced.fix_actions[0]
        .command
        .as_deref()
        .expect("fix command present");
    assert_eq!(fix_cmd_forced, "ivar feature delete checkout --yes --force");
}

#[test]
fn delete_ask_interactive_declined_deletes_nothing() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone()).with_confirm(crate::action::confirm::fixed_interactive(false));
    let layout = Layout::at(root.clone());
    create_action(
        &ctx,
        CreateInput {
            name: "child".to_owned(),
            branch: None,
            base: None,
            parent: Some("checkout".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let input = DeleteInput {
        name: "checkout".to_owned(),
        force: false,
        descendants: Descendants::Ask,
    };
    let failure = delete(&ctx, input).unwrap_err();
    assert_eq!(failure.code, "feature.delete_declined");

    assert!(
        layout
            .feature_dir(&FeatureName::new("checkout").unwrap())
            .exists()
    );
    assert!(
        layout
            .feature_dir(&FeatureName::new("child").unwrap())
            .exists()
    );
}

#[test]
fn delete_ask_interactive_yes_passes_rendered_tree_and_deletes() {
    let (_guard, root) = hall_with_promoted_feature();
    let recording = std::sync::Arc::new(RecordingConfirm::new(true));
    let ctx = Ctx::new(root.clone()).with_confirm(recording.clone());
    let layout = Layout::at(root.clone());
    create_action(
        &ctx,
        CreateInput {
            name: "child".to_owned(),
            branch: None,
            base: None,
            parent: Some("checkout".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let input = DeleteInput {
        name: "checkout".to_owned(),
        force: false,
        descendants: Descendants::Ask,
    };
    let report = delete(&ctx, input).unwrap();
    assert!(report.value.feature_removed);
    assert_eq!(report.value.descendants.len(), 1);

    let question = recording.captured_question.lock().unwrap().clone().unwrap();
    let caveat = recording.captured_caveat.lock().unwrap().clone().unwrap();
    assert_eq!(question, "Delete `checkout` and its 1 subfeature(s)?");
    assert!(caveat.contains("checkout"));
    assert!(caveat.contains("└── child"));

    assert!(
        !layout
            .feature_dir(&FeatureName::new("child").unwrap())
            .exists()
    );
    assert!(
        !layout
            .feature_dir(&FeatureName::new("checkout").unwrap())
            .exists()
    );
}

#[test]
fn delete_consented_removes_three_level_tree_leaves_first() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    create_action(
        &ctx,
        CreateInput {
            name: "child".to_owned(),
            branch: None,
            base: None,
            parent: Some("checkout".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    create_action(
        &ctx,
        CreateInput {
            name: "leaf".to_owned(),
            branch: None,
            base: None,
            parent: Some("child".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let input = DeleteInput {
        name: "checkout".to_owned(),
        force: false,
        descendants: Descendants::Consented,
    };
    let report = delete(&ctx, input).unwrap();
    assert!(report.value.feature_removed);
    assert_eq!(report.value.name.as_str(), "checkout");
    assert_eq!(report.value.descendants.len(), 2);
    // Teardown order is reverse pre-order: leaf first, then child
    assert_eq!(report.value.descendants[0].name.as_str(), "leaf");
    assert_eq!(report.value.descendants[1].name.as_str(), "child");
    assert!(report.value.descendants[0].feature_removed);
    assert!(report.value.descendants[1].feature_removed);

    assert!(
        !layout
            .feature_dir(&FeatureName::new("leaf").unwrap())
            .exists()
    );
    assert!(
        !layout
            .feature_dir(&FeatureName::new("child").unwrap())
            .exists()
    );
    assert!(
        !layout
            .feature_dir(&FeatureName::new("checkout").unwrap())
            .exists()
    );
}

#[test]
fn delete_preflight_blocker_on_descendant_refuses_before_mutation() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    create_action(
        &ctx,
        CreateInput {
            name: "child".to_owned(),
            branch: None,
            base: None,
            parent: Some("checkout".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    promote::promote(
        &ctx,
        PromoteInput {
            feature: "child".to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    // Dirty worktree on child
    let draft = root.join(".ivar/repos/api/child/draft.md");
    fs::write_text(&draft, "unsaved child work\n").unwrap();

    let input = DeleteInput {
        name: "checkout".to_owned(),
        force: false,
        descendants: Descendants::Consented,
    };
    let failure = delete(&ctx, input).unwrap_err();
    assert_eq!(failure.code, "feature.delete_unsaved_work");
    assert!(
        failure
            .actual
            .as_deref()
            .unwrap()
            .contains("`child`: `api` has uncommitted or untracked changes")
    );

    // Neither child nor parent was deleted
    assert!(
        layout
            .feature_dir(&FeatureName::new("child").unwrap())
            .exists()
    );
    assert!(
        layout
            .feature_dir(&FeatureName::new("checkout").unwrap())
            .exists()
    );
    assert!(fs::is_file(&draft).unwrap());
}

#[test]
fn delete_stop_on_failure_preserves_failing_node_and_ancestors() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    create_action(
        &ctx,
        CreateInput {
            name: "child".to_owned(),
            branch: None,
            base: None,
            parent: Some("checkout".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    create_action(
        &ctx,
        CreateInput {
            name: "leaf".to_owned(),
            branch: None,
            base: None,
            parent: Some("child".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    promote::promote(
        &ctx,
        PromoteInput {
            feature: "child".to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    // Lock child's worktree to make remove_worktree_entry fail
    let worktree = root.join(".ivar/repos/api/child");
    crate::test_support::git(
        &root,
        &[
            "--git-dir",
            root.join(".ivar/repos/api/.bare").as_str(),
            "worktree",
            "lock",
            worktree.as_str(),
        ],
    );

    let input = DeleteInput {
        name: "checkout".to_owned(),
        force: false,
        descendants: Descendants::Consented,
    };
    let report = delete(&ctx, input).unwrap();

    // Root feature not removed
    assert!(!report.value.feature_removed);
    // Descendants reported: leaf was removed, child failed and preserved
    assert_eq!(report.value.descendants.len(), 2);
    assert_eq!(report.value.descendants[0].name.as_str(), "leaf");
    assert!(report.value.descendants[0].feature_removed);
    assert_eq!(report.value.descendants[1].name.as_str(), "child");
    assert!(!report.value.descendants[1].feature_removed);
    assert!(!report.value.descendants[1].worktrees[0].removed);

    // Deepest leaf was deleted before child failed
    assert!(
        !layout
            .feature_dir(&FeatureName::new("leaf").unwrap())
            .exists()
    );
    // Child record and worktree preserved
    assert!(
        layout
            .feature_dir(&FeatureName::new("child").unwrap())
            .exists()
    );
    assert!(fs::is_dir(&worktree).unwrap());
    // Parent checkout record preserved
    assert!(
        layout
            .feature_dir(&FeatureName::new("checkout").unwrap())
            .exists()
    );
}

#[test]
fn delete_leaf_with_ask_never_prompts() {
    let (_guard, root) = hall_with_promoted_feature();
    // NonInteractive confirm
    let ctx = Ctx::new(root.clone());
    let input = DeleteInput {
        name: "checkout".to_owned(),
        force: false,
        descendants: Descendants::Ask,
    };
    let report = delete(&ctx, input).unwrap();
    assert!(report.value.feature_removed);
    assert!(report.value.descendants.is_empty());
}

#[test]
fn fold_into_ancestors_removes_descendants_preserving_order() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    create_action(
        &ctx,
        CreateInput {
            name: "child".to_owned(),
            branch: None,
            base: None,
            parent: Some("checkout".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    create_action(
        &ctx,
        CreateInput {
            name: "leaf".to_owned(),
            branch: None,
            base: None,
            parent: Some("child".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    create_action(
        &ctx,
        CreateInput {
            name: "other".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let targets = vec!["leaf".to_owned(), "checkout".to_owned(), "other".to_owned()];
    let folded = fold_into_ancestors(&ctx, &targets).unwrap();
    // "leaf" is a descendant of "checkout", so it is omitted; "checkout", "other" are kept in order
    assert_eq!(folded, vec!["checkout", "other"]);
}
