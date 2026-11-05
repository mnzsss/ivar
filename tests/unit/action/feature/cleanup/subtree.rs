use super::*;

#[test]
fn preview_lists_descendants_depth_first() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    add_subfeature(&root, "child-a", "checkout", true);
    add_subfeature(&root, "child-b", "checkout", false);
    add_subfeature(&root, "grandchild", "child-a", true);

    let preview = run_preview(&root);

    let names: Vec<String> = preview
        .descendants
        .iter()
        .map(|d| d.feature.as_str().to_owned())
        .collect();
    assert_eq!(names, vec!["child-a", "grandchild", "child-b"]);
    assert_eq!(preview.descendants[0].depth, 1);
    assert_eq!(preview.descendants[1].depth, 2);
    assert_eq!(preview.descendants[2].depth, 1);
    assert_eq!(
        preview.descendants[0].parent,
        FeatureName::new("checkout").unwrap()
    );
    assert_eq!(
        preview.descendants[1].parent,
        FeatureName::new("child-a").unwrap()
    );
}

#[test]
fn preview_reports_descendant_blockers() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    add_subfeature(&root, "child", "checkout", true);

    let preview = run_preview(&root);
    assert!(preview.has_blockers());
    assert!(preview.blockers.is_empty());
    assert_eq!(
        preview.descendants[0].blockers,
        vec![CleanupBlocker::NotIntegrated {
            state: crate::domain::feature::FeatureIntegrationState::Active,
        }]
    );
}

#[test]
fn apply_tears_down_whole_subtree_leaves_first() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    add_subfeature(&root, "child", "checkout", true);

    // Abandon the child so it becomes eligible
    close_feature(
        &Ctx::new(root.clone()),
        CloseInput {
            name: "child".to_owned(),
            outcome: "abandoned".to_owned(),
        },
    )
    .unwrap();
    let preview = run_preview(&root);
    assert!(!preview.has_blockers());
    let apply = apply_with_approved_record(&root);
    assert!(apply.feature_removed);
    assert_eq!(apply.descendants.len(), 1);
    assert_eq!(apply.descendants[0].feature.as_str(), "child");
    assert!(apply.descendants[0].feature_removed);
    assert!(!root.join(".ivar/features/checkout").exists());
    assert!(!root.join(".ivar/features/child").exists());
}

#[test]
fn apply_refuses_when_descendant_is_not_integrated() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    add_subfeature(&root, "child", "checkout", true);

    let preview = run_preview(&root);
    assert!(preview.has_blockers());
    let layout = Layout::at(root.clone());
    fs::ensure_dir(&layout.update_pages_dir()).unwrap();
    let record_rel_path = Utf8PathBuf::from("docs/memory/update/001-checkout.cleanup.json");
    fs::write_text(
        &root.join(&record_rel_path),
        &format!(
            r#"{{
                "schema_version": 1,
                "feature": "checkout",
                "branch": "checkout",
                "fingerprint": "{}",
                "approvals": {{
                    "delivery": {{ "approved": true, "at": "2026-08-28T12:00:00Z" }},
                    "documentation": {{ "decision": "written", "paths": ["docs/memory/product/001-checkout.md"], "reason": null, "at": "2026-08-28T12:05:00Z" }},
                    "teardown": {{ "approved": true, "at": "2026-08-28T12:10:00Z" }}
                }},
                "outcome": null
            }}"#,
            preview.fingerprint
        ),
    )
    .unwrap();
    let result = cleanup(
        &Ctx::new(root.clone()),
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: false,
            record: Some(record_rel_path),
            session_id: None,
        },
    );
    assert!(result.is_err());
}

#[test]
fn apply_stops_at_first_failed_node_and_leaves_ancestor_feature_json() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    add_subfeature(&root, "child", "checkout", true);

    close_feature(
        &Ctx::new(root.clone()),
        CloseInput {
            name: "child".to_owned(),
            outcome: "abandoned".to_owned(),
        },
    )
    .unwrap();
    let preview = run_preview(&root);
    let layout = Layout::at(root.clone());
    fs::ensure_dir(&layout.update_pages_dir()).unwrap();
    let record_rel_path = Utf8PathBuf::from("docs/memory/update/001-checkout.cleanup.json");
    fs::write_text(
        &root.join(&record_rel_path),
        &format!(
            r#"{{
                "schema_version": 1,
                "feature": "checkout",
                "branch": "checkout",
                "fingerprint": "{}",
                "approvals": {{
                    "delivery": {{ "approved": true, "at": "2026-08-28T12:00:00Z" }},
                    "documentation": {{ "decision": "written", "paths": ["docs/memory/product/001-checkout.md"], "reason": null, "at": "2026-08-28T12:05:00Z" }},
                    "teardown": {{ "approved": true, "at": "2026-08-28T12:10:00Z" }}
                }},
                "outcome": null
            }}"#,
            preview.fingerprint
        ),
    )
    .unwrap();

    let child = crate::action::feature::relations::read_feature(
        &layout,
        &FeatureName::new("child").unwrap(),
    )
    .unwrap();
    let child_worktree = layout.repo_worktree(&RepoName::new("api").unwrap(), &child.branch);
    let blocker_file = child_worktree.join(".locked_file");
    fs::write_text(&blocker_file, "locked").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(child_worktree.as_std_path())
            .unwrap()
            .permissions();
        perms.set_mode(0o555);
        std::fs::set_permissions(child_worktree.as_std_path(), perms).unwrap();
    }

    let outcome = cleanup(
        &Ctx::new(root.clone()),
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: false,
            record: Some(record_rel_path),
            session_id: None,
        },
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(child_worktree.as_std_path())
            .unwrap()
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(child_worktree.as_std_path(), perms).unwrap();
    }

    if let Ok(report) = outcome {
        let apply = report.value.apply_outcome.unwrap();
        assert!(
            !apply.feature_removed,
            "root feature.json must be kept when child fails"
        );
        assert!(root.join(".ivar/features/checkout").exists());
    }
}

#[test]
fn apply_updates_durable_record_file_with_descendants_outcome() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    add_subfeature(&root, "child", "checkout", true);

    let _layout = Layout::at(root.clone());
    close_feature(
        &Ctx::new(root.clone()),
        CloseInput {
            name: "child".to_owned(),
            outcome: "abandoned".to_owned(),
        },
    )
    .unwrap();

    let _apply = apply_with_approved_record(&root);

    let record_file = root.join("docs/memory/update/001-checkout.cleanup.json");
    let text = fs::read_text(&record_file).unwrap().unwrap();
    let record: CleanupRecord = serde_json::from_str(&text).unwrap();
    let outcome = record.outcome.unwrap();
    assert_eq!(outcome.descendants.len(), 1);
    assert_eq!(outcome.descendants[0].feature.as_str(), "child");
    assert!(outcome.descendants[0].feature_removed);
}
