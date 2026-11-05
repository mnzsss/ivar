use super::*;

#[test]
fn record_pointing_outside_update_pages_refused() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    fs::ensure_dir(&layout.update_pages_dir()).unwrap();
    let record_file = layout
        .page_dir(PageKind::Product)
        .join("001-checkout.cleanup.json");
    fs::ensure_dir(record_file.parent().unwrap()).unwrap();
    fs::write_text(
        &record_file,
        r#"{
            "schema_version": 1, "feature": "checkout", "branch": "checkout", "fingerprint": "1234",
            "approvals": {
                "delivery": { "approved": true, "at": "2026-08-28T12:00:00Z" },
                "documentation": { "decision": "not_required", "paths": [], "reason": "Refactoring", "at": "2026-08-28T12:05:00Z" },
                "teardown": { "approved": true, "at": "2026-08-28T12:10:00Z" }
            },
            "outcome": null
        }"#,
    )
    .unwrap();

    let err = cleanup(
        &ctx,
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: false,
            record: Some(Utf8PathBuf::from(
                "docs/memory/product/001-checkout.cleanup.json",
            )),
            session_id: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code, "feature.cleanup_record_outside_docs_updates");
}

#[test]
fn fully_valid_record_executes_teardown_removes_local_branches_and_writes_outcome() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let name = FeatureName::new("checkout").unwrap();
    let branch = BranchName::new("checkout").unwrap();
    let repo = RepoName::new("api").unwrap();
    let bare = layout.repo_bare(&repo);
    let worktree = layout.repo_worktree(&repo, &branch);
    let plan_dir = layout.plan_dir(&name);
    let feature_dir = layout.feature_dir(&name);
    let git_sys = git::System;

    // Verify initial state
    fs::ensure_dir(&plan_dir).unwrap();
    assert!(fs::is_dir(&worktree).unwrap());
    assert!(fs::is_dir(&plan_dir).unwrap());
    assert!(fs::is_dir(&feature_dir).unwrap());
    assert!(git_sys.revision_commit(&bare, "checkout").is_ok());

    // 1. Get preview to read canonical fingerprint
    let preview = run_preview(&root);
    assert!(preview.blockers.is_empty());

    // 2. Write valid record under docs/memory/update/
    fs::ensure_dir(&layout.update_pages_dir()).unwrap();
    let record_rel_path = Utf8PathBuf::from("docs/memory/update/001-checkout.cleanup.json");
    let record_file = root.join(&record_rel_path);
    fs::write_text(
        &record_file,
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

    // 3. Apply cleanup with record
    let outcome = cleanup(
        &ctx,
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: false,
            record: Some(record_rel_path.clone()),
            session_id: None,
        },
    )
    .unwrap();

    let apply = outcome.value.apply_outcome.expect("expected apply outcome");
    assert!(apply.feature_removed);
    assert_eq!(apply.worktrees.len(), 1);
    assert!(apply.worktrees[0].removed);
    assert_eq!(apply.branches.len(), 1);
    assert!(apply.branches[0].deleted);

    // Verify filesystem and git teardown
    assert!(!fs::is_dir(&worktree).unwrap());
    assert!(!fs::is_dir(&plan_dir).unwrap());
    assert!(!fs::is_dir(&feature_dir).unwrap());
    assert!(git_sys.revision_commit(&bare, "checkout").is_err());
    assert!(Feature::read(&layout, &name).unwrap().is_none());

    // Verify record updated with outcome
    assert!(fs::is_file(&record_file).unwrap());
    let content = fs::read_text(&record_file).unwrap().unwrap();
    let record: CleanupRecord = serde_json::from_str(&content).unwrap();
    let recorded_outcome = record.outcome.expect("expected recorded outcome");
    assert!(recorded_outcome.feature_removed);
    assert!(recorded_outcome.worktrees[0].removed);
    assert!(recorded_outcome.branches[0].deleted);

    // Re-running apply on completed record is refused
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

    assert_eq!(err.code, "feature.cleanup_record_invalid");
}

#[test]
fn record_fingerprint_mismatch_refused() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());

    fs::ensure_dir(&layout.update_pages_dir()).unwrap();
    let record_file = layout.update_pages_dir().join("001-checkout.cleanup.json");
    fs::write_text(
        &record_file,
        r#"{
            "schema_version": 1,
            "feature": "checkout",
            "branch": "checkout",
            "fingerprint": "bad_fingerprint_hex",
            "approvals": {
                "delivery": { "approved": true, "at": "2026-08-28T12:00:00Z" },
                "documentation": { "decision": "not_required", "paths": [], "reason": "Internal refactor", "at": "2026-08-28T12:05:00Z" },
                "teardown": { "approved": true, "at": "2026-08-28T12:10:00Z" }
            },
            "outcome": null
        }"#,
    )
    .unwrap();

    let err = cleanup(
        &ctx,
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: false,
            record: Some(Utf8PathBuf::from(
                "docs/memory/update/001-checkout.cleanup.json",
            )),
            session_id: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code, "feature.cleanup_fingerprint_mismatch");
    assert!(
        !err.fix_actions
            .iter()
            .any(|fix| fix.code == "feature.cleanup_forge_moved"),
        "a feature local git can judge must not blame the forge: {err:?}"
    );
}

#[test]
fn record_unapproved_delivery_or_teardown_refused() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());

    let preview = run_preview(&root);

    fs::ensure_dir(&layout.update_pages_dir()).unwrap();
    let record_file = layout.update_pages_dir().join("001-checkout.cleanup.json");
    fs::write_text(
        &record_file,
        &format!(
            r#"{{
                "schema_version": 1,
                "feature": "checkout",
                "branch": "checkout",
                "fingerprint": "{}",
                "approvals": {{
                    "delivery": {{ "approved": false, "at": "2026-08-28T12:00:00Z" }},
                    "documentation": {{ "decision": "not_required", "paths": [], "reason": "Internal refactor", "at": "2026-08-28T12:05:00Z" }},
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
            record: Some(Utf8PathBuf::from(
                "docs/memory/update/001-checkout.cleanup.json",
            )),
            session_id: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code, "feature.cleanup_delivery_not_approved");

    // Test teardown unapproved
    fs::write_text(
        &record_file,
        &format!(
            r#"{{
                "schema_version": 1,
                "feature": "checkout",
                "branch": "checkout",
                "fingerprint": "{}",
                "approvals": {{
                    "delivery": {{ "approved": true, "at": "2026-08-28T12:00:00Z" }},
                    "documentation": {{ "decision": "not_required", "paths": [], "reason": "Internal refactor", "at": "2026-08-28T12:05:00Z" }},
                    "teardown": {{ "approved": false, "at": "2026-08-28T12:10:00Z" }}
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
            record: Some(Utf8PathBuf::from(
                "docs/memory/update/001-checkout.cleanup.json",
            )),
            session_id: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code, "feature.cleanup_teardown_not_approved");
}

#[test]
fn record_feature_mismatch_refused() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());

    let preview = run_preview(&root);

    fs::ensure_dir(&layout.update_pages_dir()).unwrap();
    let record_file = layout.update_pages_dir().join("001-checkout.cleanup.json");
    fs::write_text(
        &record_file,
        &format!(
            r#"{{
                "schema_version": 1,
                "feature": "other-feature",
                "branch": "checkout",
                "fingerprint": "{}",
                "approvals": {{
                    "delivery": {{ "approved": true, "at": "2026-08-28T12:00:00Z" }},
                    "documentation": {{ "decision": "not_required", "paths": [], "reason": "Internal refactor", "at": "2026-08-28T12:05:00Z" }},
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
            record: Some(Utf8PathBuf::from(
                "docs/memory/update/001-checkout.cleanup.json",
            )),
            session_id: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code, "feature.cleanup_record_feature_mismatch");
}
