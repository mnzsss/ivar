#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

fn repo_facts() -> CleanupRepoFacts {
    CleanupRepoFacts {
        repo: RepoName::new("api").unwrap(),
        effective_base: Some(BranchName::new("main").unwrap()),
        feature_head: Some("feature".to_owned()),
        base_head: Some("base".to_owned()),
        local_branch_exists: true,
        worktree_exists: true,
        clone_exists: true,
        dirty_worktree: Some(false),
        unmerged_commits: Some(0),
        in_manifest: true,
        inspection_error: None,
    }
}

#[test]
fn merged_repo_is_eligible() {
    let verdict = classify_cleanup(&CleanupFacts {
        repos: vec![repo_facts()],
        live_sessions: Vec::new(),
        descendants: Vec::new(),
        session_inspection_error: None,
    });

    assert!(verdict.eligible());
    assert!(verdict.blockers.is_empty());
}

#[test]
fn collects_all_repository_blockers() {
    let mut facts = repo_facts();
    facts.clone_exists = false;
    facts.worktree_exists = false;
    facts.dirty_worktree = Some(true);
    facts.unmerged_commits = Some(2);
    let verdict = classify_cleanup(&CleanupFacts {
        repos: vec![facts],
        live_sessions: vec![SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap()],
        descendants: vec![FeatureName::new("child").unwrap()],
        session_inspection_error: None,
    });

    assert_eq!(verdict.blockers.len(), 6);
    assert!(
        verdict
            .blockers
            .iter()
            .any(|blocker| matches!(blocker, CleanupBlocker::LiveSessions { .. }))
    );
    assert!(
        verdict
            .blockers
            .iter()
            .any(|blocker| matches!(blocker, CleanupBlocker::Descendants { .. }))
    );
    assert!(
        verdict
            .blockers
            .iter()
            .any(|blocker| matches!(blocker, CleanupBlocker::MissingClone { .. }))
    );
    assert!(
        verdict
            .blockers
            .iter()
            .any(|blocker| matches!(blocker, CleanupBlocker::MissingWorktree { .. }))
    );
    assert!(
        verdict
            .blockers
            .iter()
            .any(|blocker| matches!(blocker, CleanupBlocker::DirtyWorktree { .. }))
    );
    assert!(
        verdict
            .blockers
            .iter()
            .any(|blocker| matches!(blocker, CleanupBlocker::UnmergedCommits { commits: 2, .. }))
    );
}

#[test]
fn empty_feature_is_explicitly_eligible() {
    let verdict = classify_cleanup(&CleanupFacts {
        repos: Vec::new(),
        live_sessions: Vec::new(),
        descendants: Vec::new(),
        session_inspection_error: None,
    });

    assert_eq!(verdict.blockers, vec![CleanupBlocker::EmptyFeature]);
    assert!(verdict.eligible());
}

#[test]
fn repo_absent_from_manifest_blocks_cleanup() {
    let mut facts = repo_facts();
    facts.in_manifest = false;
    let verdict = classify_cleanup(&CleanupFacts {
        repos: vec![facts],
        live_sessions: Vec::new(),
        descendants: Vec::new(),
        session_inspection_error: None,
    });

    assert_eq!(
        verdict.blockers,
        vec![CleanupBlocker::RepoAbsentFromManifest {
            repo: RepoName::new("api").unwrap(),
        }]
    );
}

#[test]
fn cleanup_preview_roundtrips_serde() {
    use camino::Utf8PathBuf;

    let preview = CleanupPreview {
        feature: FeatureName::new("checkout").unwrap(),
        branch: BranchName::new("checkout").unwrap(),
        repos: Vec::new(),
        blockers: vec![CleanupBlocker::EmptyFeature],
        paths_to_remove: vec![
            Utf8PathBuf::from("plans/checkout"),
            Utf8PathBuf::from(".ivar/features/checkout"),
        ],
        fingerprint: "sha256:1234".to_owned(),
    };

    let serialized = serde_json::to_string(&preview).unwrap();
    let deserialized: CleanupPreview = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized, preview);
}

fn sample_record() -> CleanupRecord {
    CleanupRecord {
        schema_version: 1,
        feature: FeatureName::new("feature-cleanup").unwrap(),
        branch: BranchName::new("feature-cleanup").unwrap(),
        fingerprint: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
        approvals: CleanupApprovals {
            delivery: DeliveryApproval {
                approved: true,
                at: "2026-08-28T12:00:00Z".to_owned(),
            },
            documentation: DocumentationApproval {
                decision: DocumentationDecision::Written,
                paths: vec![
                    Utf8PathBuf::from("docs/product/003-x.md"),
                    Utf8PathBuf::from("docs/updates/007-y.md"),
                ],
                reason: None,
                at: "2026-08-28T12:05:00Z".to_owned(),
            },
            teardown: TeardownApproval {
                approved: true,
                at: "2026-08-28T12:10:00Z".to_owned(),
            },
        },
        outcome: None,
    }
}

#[test]
fn cleanup_record_serde_roundtrip() {
    let record = sample_record();
    let json = serde_json::to_string(&record).unwrap();
    let deserialized: CleanupRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(record, deserialized);
    assert!(record.validate().is_ok());
}

#[test]
fn cleanup_record_rejects_unsupported_schema_version() {
    let mut record = sample_record();
    record.schema_version = 2;
    let err = record.validate().unwrap_err();
    assert!(err.contains("unsupported schema_version `2`"));
}

#[test]
fn cleanup_record_documentation_written_rules() {
    let mut record = sample_record();
    record.approvals.documentation.paths = Vec::new();
    assert!(
        record
            .validate()
            .unwrap_err()
            .contains("requires non-empty `paths`")
    );

    let mut record = sample_record();
    record.approvals.documentation.reason = Some("reason".to_owned());
    assert!(
        record
            .validate()
            .unwrap_err()
            .contains("requires null `reason`")
    );
}

#[test]
fn cleanup_record_documentation_not_required_rules() {
    let mut record = sample_record();
    record.approvals.documentation.decision = DocumentationDecision::NotRequired;
    record.approvals.documentation.reason = Some("Internal refactoring".to_owned());
    record.approvals.documentation.paths = Vec::new();
    assert!(record.validate().is_ok());

    let mut record = sample_record();
    record.approvals.documentation.decision = DocumentationDecision::NotRequired;
    record.approvals.documentation.reason = Some("Internal refactoring".to_owned());
    record.approvals.documentation.paths = vec![Utf8PathBuf::from("docs/product/003-x.md")];
    assert!(
        record
            .validate()
            .unwrap_err()
            .contains("requires empty `paths`")
    );

    let mut record = sample_record();
    record.approvals.documentation.decision = DocumentationDecision::NotRequired;
    record.approvals.documentation.reason = None;
    record.approvals.documentation.paths = Vec::new();
    assert!(
        record
            .validate()
            .unwrap_err()
            .contains("requires a non-empty `reason`")
    );
}

#[test]
fn cleanup_record_rejects_non_hall_relative_documentation_paths() {
    let mut record = sample_record();
    record.approvals.documentation.paths = vec![Utf8PathBuf::from("plans/x/plan.md")];
    assert!(record.validate().unwrap_err().contains("not hall-relative"));

    let mut record = sample_record();
    record.approvals.documentation.paths = vec![Utf8PathBuf::from("docs/../plans/x/plan.md")];
    assert!(record.validate().unwrap_err().contains("not hall-relative"));
}

#[test]
fn cleanup_record_rejects_populated_outcome() {
    let mut record = sample_record();
    record.outcome = Some(CleanupApplyOutcome {
        feature: FeatureName::new("feature-cleanup").unwrap(),
        branch: BranchName::new("feature-cleanup").unwrap(),
        fingerprint: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
        worktrees: Vec::new(),
        branches: Vec::new(),
        feature_removed: true,
        plans_removed: true,
    });
    assert!(
        record
            .validate()
            .unwrap_err()
            .contains("outcome is already populated")
    );
}

#[test]
fn cleanup_record_rejects_unknown_fields() {
    let json = r#"{
        "schema_version": 1,
        "feature": "feature-cleanup",
        "branch": "feature-cleanup",
        "fingerprint": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "approvals": {
            "delivery": { "approved": true, "at": "2026-08-28T12:00:00Z" },
            "documentation": { "decision": "not_required", "paths": [], "reason": "internal", "at": "2026-08-28T12:05:00Z" },
            "teardown": { "approved": true, "at": "2026-08-28T12:10:00Z" }
        },
        "outcome": null,
        "unexpected_field": 123
    }"#;
    assert!(serde_json::from_str::<CleanupRecord>(json).is_err());
}
