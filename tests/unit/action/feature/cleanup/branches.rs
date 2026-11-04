use super::*;

#[test]
fn apply_cleanup_reports_branch_deletion_in_apply_outcome() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let repo = RepoName::new("api").unwrap();
    let bare = layout.repo_bare(&repo);
    let git_sys = git::System;

    let preview = run_preview(&root);
    assert!(preview.blockers.is_empty());
    assert!(git_sys.revision_commit(&bare, "checkout").is_ok());

    fs::ensure_dir(&layout.docs_updates_dir()).unwrap();
    let record_rel_path = Utf8PathBuf::from("docs/updates/001-checkout.cleanup.json");
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
                    "documentation": {{ "decision": "not_required", "paths": [], "reason": "Internal refactor", "at": "2026-08-28T12:05:00Z" }},
                    "teardown": {{ "approved": true, "at": "2026-08-28T12:10:00Z" }}
                }},
                "outcome": null
            }}"#,
            preview.fingerprint
        ),
    )
    .unwrap();

    let outcome = cleanup(
        &ctx,
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: false,
            record: Some(record_rel_path),
            session_id: None,
        },
    )
    .unwrap();

    let apply = outcome.value.apply_outcome.unwrap();
    assert!(apply.feature_removed);
    assert_eq!(apply.branches.len(), 1);
    assert!(apply.branches[0].deleted);
    assert!(git_sys.revision_commit(&bare, "checkout").is_err());
}

#[test]
fn apply_cleanup_preserves_remote_refs() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let repo = RepoName::new("api").unwrap();
    let bare = layout.repo_bare(&repo);
    let git_sys = git::System;

    // Create a mock remote tracking branch in bare repo
    let head_commit = git_sys.revision_commit(&bare, "checkout").unwrap();
    test_git(
        &bare,
        &["update-ref", "refs/remotes/origin/checkout", &head_commit],
    );
    let remote_commit_before = git_sys
        .revision_commit(&bare, "refs/remotes/origin/checkout")
        .unwrap();

    let preview = run_preview(&root);
    fs::ensure_dir(&layout.docs_updates_dir()).unwrap();
    let record_rel_path = Utf8PathBuf::from("docs/updates/001-checkout.cleanup.json");
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
                    "documentation": {{ "decision": "not_required", "paths": [], "reason": "Internal refactor", "at": "2026-08-28T12:05:00Z" }},
                    "teardown": {{ "approved": true, "at": "2026-08-28T12:10:00Z" }}
                }},
                "outcome": null
            }}"#,
            preview.fingerprint
        ),
    )
    .unwrap();

    cleanup(
        &ctx,
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: false,
            record: Some(record_rel_path),
            session_id: None,
        },
    )
    .unwrap();

    let remote_commit_after = git_sys
        .revision_commit(&bare, "refs/remotes/origin/checkout")
        .unwrap();
    assert_eq!(remote_commit_before, remote_commit_after);
}
