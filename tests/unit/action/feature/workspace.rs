#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use crate::action::Ctx;
use crate::action::feature::create::{CreateInput, create as create_action};
use crate::action::feature::promote::{PromoteInput, promote};
use crate::action::feature::workspace::{
    WorkspaceFolderOutcome, WorkspaceInput, WorkspaceOutcome, workspace,
};
use crate::action::hall::{self, InitInput};
use crate::action::sync::sync;
use crate::domain::name::{BranchName, FeatureName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::error::{Status, WriteHuman};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

fn multi_repo_hall_with_feature() -> (tempfile::TempDir, Utf8PathBuf) {
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

    let origins = root.parent().unwrap().join("origins");
    let origin_api = seeded_repo(&origins.join("api"), "main");
    let origin_web = seeded_repo(&origins.join("web"), "main");
    let origin_docs = seeded_repo(&origins.join("docs"), "master");

    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![
            Repo::new(
                RepoName::new("api").unwrap(),
                origin_api.as_str(),
                BranchName::new("main").unwrap(),
            ),
            Repo::new(
                RepoName::new("web").unwrap(),
                origin_web.as_str(),
                BranchName::new("main").unwrap(),
            ),
            Repo::new(
                RepoName::new("docs").unwrap(),
                origin_docs.as_str(),
                BranchName::new("master").unwrap(),
            ),
        ],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

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

    sync(&ctx, Default::default()).unwrap();

    promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    (guard, root)
}

#[test]
fn workspace_generates_code_workspace_with_promoted_and_readonly_context_folders() {
    let (_guard, root) = multi_repo_hall_with_feature();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());

    let report = workspace(
        &ctx,
        WorkspaceInput {
            feature: "checkout".to_owned(),
            repos: vec![],
        },
    )
    .unwrap();

    assert!(report.is_clean());
    let expected_path = layout.feature_workspace(&FeatureName::new("checkout").unwrap());
    assert_eq!(report.value.path, expected_path);
    assert!(expected_path.is_file());

    let content = fs::read_text(&expected_path).unwrap().unwrap();
    let doc: serde_json::Value = serde_json::from_str(&content).unwrap();

    // Promoted repo 'api' on checkout branch, non-promoted 'web' on main, 'docs' on master
    let expected_api_path = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    let expected_web_path = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let expected_docs_path = layout.repo_worktree(
        &RepoName::new("docs").unwrap(),
        &BranchName::new("master").unwrap(),
    );

    let folders = doc["folders"].as_array().unwrap();
    assert_eq!(folders.len(), 3);
    assert_eq!(folders[0]["path"], expected_api_path.as_str());
    assert_eq!(folders[0]["name"], "api");
    assert_eq!(folders[1]["path"], expected_web_path.as_str());
    assert_eq!(folders[1]["name"], "web");
    assert_eq!(folders[2]["path"], expected_docs_path.as_str());
    assert_eq!(folders[2]["name"], "docs");

    // readonlyInclude contains web and docs with /** suffix, but not api
    let readonly = doc["settings"]["files.readonlyInclude"].as_object().unwrap();
    assert_eq!(readonly.len(), 2);
    let web_key = format!("{expected_web_path}/**");
    let docs_key = format!("{expected_docs_path}/**");
    assert_eq!(readonly.get(&web_key), Some(&serde_json::Value::Bool(true)));
    assert_eq!(readonly.get(&docs_key), Some(&serde_json::Value::Bool(true)));
    let api_key = format!("{expected_api_path}/**");
    assert!(!readonly.contains_key(&api_key));

    // Check Outcome contents
    assert_eq!(report.value.folders.len(), 3);
    assert_eq!(
        report.value.folders[0],
        WorkspaceFolderOutcome {
            repo: RepoName::new("api").unwrap(),
            branch: BranchName::new("checkout").unwrap(),
            path: expected_api_path,
            readonly: false,
        }
    );
    assert_eq!(
        report.value.folders[1],
        WorkspaceFolderOutcome {
            repo: RepoName::new("web").unwrap(),
            branch: BranchName::new("main").unwrap(),
            path: expected_web_path,
            readonly: true,
        }
    );
    assert_eq!(
        report.value.folders[2],
        WorkspaceFolderOutcome {
            repo: RepoName::new("docs").unwrap(),
            branch: BranchName::new("master").unwrap(),
            path: expected_docs_path,
            readonly: true,
        }
    );
}

#[test]
fn workspace_filters_repos_and_preserves_manifest_order() {
    let (_guard, root) = multi_repo_hall_with_feature();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root);

    // Pass repos in reverse order: docs then api
    let report = workspace(
        &ctx,
        WorkspaceInput {
            feature: "checkout".to_owned(),
            repos: vec!["docs".to_owned(), "api".to_owned()],
        },
    )
    .unwrap();

    // Manifest order is api, web, docs -> selected repos must be api, docs
    assert_eq!(report.value.folders.len(), 2);
    assert_eq!(report.value.folders[0].repo.as_str(), "api");
    assert!(!report.value.folders[0].readonly);
    assert_eq!(report.value.folders[1].repo.as_str(), "docs");
    assert!(report.value.folders[1].readonly);

    let doc: serde_json::Value =
        serde_json::from_str(&fs::read_text(&report.value.path).unwrap().unwrap()).unwrap();
    let folders = doc["folders"].as_array().unwrap();
    assert_eq!(folders.len(), 2);
    assert_eq!(folders[0]["name"], "api");
    assert_eq!(folders[1]["name"], "docs");

    let readonly = doc["settings"]["files.readonlyInclude"].as_object().unwrap();
    assert_eq!(readonly.len(), 1);
    let expected_docs_path = layout.repo_worktree(
        &RepoName::new("docs").unwrap(),
        &BranchName::new("master").unwrap(),
    );
    assert_eq!(
        readonly.get(&format!("{expected_docs_path}/**")),
        Some(&serde_json::Value::Bool(true))
    );
}

#[test]
fn workspace_fails_when_feature_not_found() {
    let (_guard, root) = multi_repo_hall_with_feature();
    let ctx = Ctx::new(root);

    let failure = workspace(
        &ctx,
        WorkspaceInput {
            feature: "ghost".to_owned(),
            repos: vec![],
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "feature.not_found");
    assert_eq!(failure.status, Status::Blocked);
}

#[test]
fn workspace_fails_when_repo_not_in_manifest() {
    let (_guard, root) = multi_repo_hall_with_feature();
    let ctx = Ctx::new(root);

    let failure = workspace(
        &ctx,
        WorkspaceInput {
            feature: "checkout".to_owned(),
            repos: vec!["api".to_owned(), "unknown_repo".to_owned()],
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "repo.not_in_manifest");
    assert_eq!(failure.status, Status::Blocked);
}

#[test]
fn workspace_is_deterministic_across_repeated_runs() {
    let (_guard, root) = multi_repo_hall_with_feature();
    let ctx = Ctx::new(root.clone());

    let report1 = workspace(
        &ctx,
        WorkspaceInput {
            feature: "checkout".to_owned(),
            repos: vec![],
        },
    )
    .unwrap();
    let content1 = fs::read_text(&report1.value.path).unwrap().unwrap();

    let report2 = workspace(
        &ctx,
        WorkspaceInput {
            feature: "checkout".to_owned(),
            repos: vec![],
        },
    )
    .unwrap();
    let content2 = fs::read_text(&report2.value.path).unwrap().unwrap();

    assert_eq!(content1, content2);
}

#[test]
fn workspace_outcome_writes_human_readable_summary() {
    let (_guard, root) = multi_repo_hall_with_feature();
    let ctx = Ctx::new(root.clone());

    let report = workspace(
        &ctx,
        WorkspaceInput {
            feature: "checkout".to_owned(),
            repos: vec![],
        },
    )
    .unwrap();

    let mut out = Vec::new();
    report.value.write_human(&mut out).unwrap();
    let rendered = String::from_utf8(out).unwrap();

    assert!(rendered.contains("Wrote workspace for `checkout` to"));
    assert!(rendered.contains("checkout.code-workspace"));
    assert!(rendered.contains("api (checkout, writable)"));
    assert!(rendered.contains("web (main, read-only)"));
    assert!(rendered.contains("docs (master, read-only)"));
}
