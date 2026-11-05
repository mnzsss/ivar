use super::*;

#[test]
fn apply_removes_a_worktree_whose_dir_differs_from_the_branch() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let bare = root.join(".ivar/repos/api/.bare");
    let moved = root.join(".ivar/repos/api/elsewhere");
    git::System
        .move_worktree(&bare, &root.join(".ivar/repos/api/checkout"), &moved)
        .unwrap();

    let apply = apply_with_approved_record(&root);

    assert!(apply.feature_removed);
    assert!(apply.worktrees[0].removed);
    assert!(!fs::exists(&moved).unwrap());
}

#[test]
fn preview_counts_a_worktree_whose_directory_is_gone_as_missing() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    remove_dir_all(root.join(".ivar/repos/api/checkout")).unwrap();

    let preview = run_preview(&root);

    assert!(!preview.repos[0].worktree_exists);
    assert!(
        preview
            .blockers
            .iter()
            .any(|blocker| matches!(blocker, CleanupBlocker::MissingWorktree { .. }))
    );
}

#[test]
fn a_blocked_cleanup_names_the_preview_command() {
    let (_guard, root) = hall_with_feature(&[], None);
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let preview = run_preview(&root);
    assert_eq!(preview.blockers, vec![CleanupBlocker::EmptyFeature]);
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
                    "documentation": {{ "decision": "not_required", "paths": [], "reason": "Refactoring", "at": "2026-08-28T12:05:00Z" }},
                    "teardown": {{ "approved": true, "at": "2026-08-28T12:10:00Z" }}
                }},
                "outcome": null
            }}"#,
            preview.fingerprint
        ),
    )
    .unwrap();

    let err = cleanup(
        &ctx,
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: false,
            record: Some(record_rel_path),
            session_id: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code, "feature.cleanup_blocked");
    assert!(
        err.fix_actions
            .iter()
            .any(|fix| fix.command.as_deref() == Some("ivar feature cleanup checkout --preview")),
        "{err:?}"
    );
}

#[test]
fn apply_warns_when_the_graph_database_cannot_drop_the_layers() {
    let (_guard, root) = hall_with_feature(&["api"], None);
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
    std::fs::write(root.join(".ivar/graph.db"), b"not sqlite").unwrap();

    let report = cleanup(
        &Ctx::new(root.clone()),
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: false,
            record: Some(record_rel_path),
            session_id: None,
        },
    )
    .unwrap();

    assert!(
        report
            .value
            .apply_outcome
            .is_some_and(|a| a.feature_removed)
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.code == "graph.layers_not_dropped" && w.subject == "checkout"),
        "warnings: {:?}",
        report.warnings
    );
}
