#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::Ctx;
use crate::action::feature::cleanup::{CleanupInput, cleanup};
use crate::action::feature::create::{CreateInput, create as create_feature};
use crate::action::feature::promote::{PromoteInput, promote};
use crate::action::hall::{self, InitInput};
use crate::action::sync::{SyncInput, sync};
use crate::domain::feature::CleanupPreview;
use crate::domain::name::{BranchName, FeatureName, HallName, RepoName, SessionId};
use crate::domain::provider::Provider;
use crate::git::{self, Git};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{git as test_git, hall_root, seeded_repo};
use camino::Utf8PathBuf;
use std::fs::remove_dir_all;

fn hall_with_feature(repos: &[&str], branch: Option<&str>) -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
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
    let layout = Layout::at(root.clone());
    let manifest_repos = repos
        .iter()
        .map(|repo| {
            let origin = seeded_repo(&root.parent().unwrap().join("origins").join(repo), "main");
            Repo::new(
                RepoName::new(*repo).unwrap(),
                origin.as_str(),
                BranchName::new("main").unwrap(),
            )
        })
        .collect();
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        manifest_repos,
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    sync(&ctx, SyncInput::default()).unwrap();
    create_feature(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: branch.map(str::to_owned),
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    for repo in repos {
        promote(
            &ctx,
            PromoteInput {
                feature: "checkout".to_owned(),
                repo: (*repo).to_owned(),
                base: None,
            },
        )
        .unwrap();
    }
    (guard, root)
}

fn run_preview(root: &Utf8PathBuf) -> CleanupPreview {
    cleanup(
        &Ctx::new(root.clone()),
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: true,
            record: None,
        },
    )
    .unwrap()
    .value
    .preview
}

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
fn reports_missing_clone_and_unmerged_commits() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let layout = Layout::at(root.clone());
    let repo = RepoName::new("api").unwrap();
    let worktree = layout.repo_worktree(&repo, &BranchName::new("checkout").unwrap());
    test_git(
        &worktree,
        &["commit", "--allow-empty", "-m", "feature change"],
    );

    let preview = run_preview(&root);
    assert!(
        preview
            .blockers
            .iter()
            .any(|blocker| matches!(blocker, CleanupBlocker::UnmergedCommits { .. }))
    );

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
fn reports_descendants() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let ctx = Ctx::new(root.clone());
    create_feature(
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

    let preview = run_preview(&root);

    assert!(
        preview
            .blockers
            .iter()
            .any(|blocker| matches!(blocker, CleanupBlocker::Descendants { .. }))
    );
}

#[test]
fn preview_paths_to_remove_lists_worktrees_in_manifest_order_then_plan_then_feature_dir() {
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
            layout.plan_dir(&feature_name),
            layout.feature_dir(&feature_name),
        ]
    );
}

#[test]
fn empty_feature_preview_lists_plan_dir_and_feature_dir_only() {
    let (_guard, root) = hall_with_feature(&[], None);
    let layout = Layout::at(root.clone());
    let feature_name = FeatureName::new("checkout").unwrap();

    let preview = run_preview(&root);

    assert_eq!(
        preview.paths_to_remove,
        vec![
            layout.plan_dir(&feature_name),
            layout.feature_dir(&feature_name),
        ]
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
        },
    )
    .unwrap()
    .value;

    let mut buf = Vec::new();
    outcome.write_human(&mut buf).unwrap();
    let human = String::from_utf8(buf).unwrap();

    assert!(human.contains("Paths to remove:"));
    assert!(human.contains("plans/checkout"));
    assert!(human.contains(".ivar/features/checkout"));
}

#[test]
fn record_pointing_outside_docs_updates_refused() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());

    // Ensure docs/updates/ and docs/product/ exist in test hall
    fs::ensure_dir(&layout.docs_updates_dir()).unwrap();
    let record_file = layout
        .root()
        .join("docs")
        .join("product")
        .join("001-checkout.cleanup.json");
    fs::ensure_dir(record_file.parent().unwrap()).unwrap();
    fs::write_text(
        &record_file,
        r#"{
            "schema_version": 1,
            "feature": "checkout",
            "branch": "checkout",
            "fingerprint": "1234",
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
            record: Some(Utf8PathBuf::from("docs/product/001-checkout.cleanup.json")),
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

    // 2. Write valid record under docs/updates/
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
                    "documentation": {{ "decision": "written", "paths": ["docs/product/001-checkout.md"], "reason": null, "at": "2026-08-28T12:05:00Z" }},
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
        },
    )
    .unwrap();

    let apply = outcome.value.apply_outcome.expect("expected apply outcome");
    assert!(apply.feature_removed);
    assert!(apply.plans_removed);
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
    assert!(recorded_outcome.plans_removed);
    assert!(recorded_outcome.worktrees[0].removed);
    assert!(recorded_outcome.branches[0].deleted);

    // Re-running apply on completed record is refused
    let err = cleanup(
        &ctx,
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: false,
            record: Some(record_rel_path),
        },
    )
    .unwrap_err();

    assert_eq!(err.code, "feature.cleanup_record_invalid");
}

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
        },
    )
    .unwrap();

    let remote_commit_after = git_sys
        .revision_commit(&bare, "refs/remotes/origin/checkout")
        .unwrap();
    assert_eq!(remote_commit_before, remote_commit_after);
}

#[test]
fn record_fingerprint_mismatch_refused() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());

    fs::ensure_dir(&layout.docs_updates_dir()).unwrap();
    let record_file = root
        .join("docs")
        .join("updates")
        .join("001-checkout.cleanup.json");
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
            record: Some(Utf8PathBuf::from("docs/updates/001-checkout.cleanup.json")),
        },
    )
    .unwrap_err();

    assert_eq!(err.code, "feature.cleanup_fingerprint_mismatch");
}

#[test]
fn record_unapproved_delivery_or_teardown_refused() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());

    let preview = run_preview(&root);

    fs::ensure_dir(&layout.docs_updates_dir()).unwrap();
    let record_file = root
        .join("docs")
        .join("updates")
        .join("001-checkout.cleanup.json");
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
            record: Some(Utf8PathBuf::from("docs/updates/001-checkout.cleanup.json")),
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
            record: Some(Utf8PathBuf::from("docs/updates/001-checkout.cleanup.json")),
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

    fs::ensure_dir(&layout.docs_updates_dir()).unwrap();
    let record_file = root
        .join("docs")
        .join("updates")
        .join("001-checkout.cleanup.json");
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
            record: Some(Utf8PathBuf::from("docs/updates/001-checkout.cleanup.json")),
        },
    )
    .unwrap_err();

    assert_eq!(err.code, "feature.cleanup_record_feature_mismatch");
}
