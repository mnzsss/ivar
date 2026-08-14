#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::feature::create::CreateInput;
use crate::action::feature::create::create as create_action;
use crate::action::feature::promote::{self, PromoteInput};
use crate::action::hall::{self, InitInput};
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

/// A hall with two seeded repos declared and a feature created; `api` is
/// promoted, `web` is not.
fn hall_with_promoted_and_plain_repo() -> (tempfile::TempDir, Utf8PathBuf) {
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
    let api_origin = seeded_repo(&origins.join("api"), "main");
    let web_origin = seeded_repo(&origins.join("web"), "main");
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![
            Repo::new(
                RepoName::new("api").unwrap(),
                api_origin.as_str(),
                BranchName::new("main").unwrap(),
            ),
            Repo::new(
                RepoName::new("web").unwrap(),
                web_origin.as_str(),
                BranchName::new("main").unwrap(),
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
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    promote::promote(
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

fn review_input(name: &str) -> ReviewInput {
    ReviewInput {
        name: name.to_owned(),
    }
}

#[test]
fn review_writes_a_valid_workspace_with_feature_branch_folders() {
    let (_guard, root) = hall_with_promoted_and_plain_repo();
    let ctx = Ctx::new(root.clone());

    let report = review(&ctx, review_input("checkout")).unwrap();

    assert!(report.is_clean());
    let workspace_path = root.join("checkout.code-workspace");
    assert_eq!(report.value.workspace, workspace_path);
    assert!(fs::is_file(&workspace_path).unwrap());

    // The output is valid JSON in VSCode's workspace shape.
    let raw = fs::read_text(&workspace_path).unwrap().unwrap();
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let folders = value.get("folders").unwrap().as_array().unwrap();
    let paths: Vec<&str> = folders
        .iter()
        .map(|folder| folder.get("path").unwrap().as_str().unwrap())
        .collect();
    // Promoted: the feature-branch worktree. Not promoted: the default
    // branch. Both relative to the hall root.
    assert!(paths.contains(&".ivar/repos/api/checkout"));
    assert!(paths.contains(&".ivar/repos/web/main"));
    assert_eq!(paths.len(), 2);
}

#[test]
fn review_rewriting_the_workspace_is_stable() {
    let (_guard, root) = hall_with_promoted_and_plain_repo();
    let ctx = Ctx::new(root.clone());
    review(&ctx, review_input("checkout")).unwrap();

    let before = fs::read_text(&root.join("checkout.code-workspace"))
        .unwrap()
        .unwrap();
    review(&ctx, review_input("checkout")).unwrap();

    let after = fs::read_text(&root.join("checkout.code-workspace"))
        .unwrap()
        .unwrap();
    assert_eq!(
        before, after,
        "re-running review must not churn the workspace file"
    );
}

#[test]
fn review_is_rejected_for_a_missing_feature() {
    let (_guard, root) = hall_with_promoted_and_plain_repo();
    let ctx = Ctx::new(root);

    let failure = review(&ctx, review_input("ghost")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.not_found");
}

#[test]
fn the_human_surface_names_the_workspace_and_folders() {
    let outcome = ReviewOutcome {
        root: Utf8PathBuf::from("/hall"),
        feature: FeatureName::new("checkout").unwrap(),
        workspace: Utf8PathBuf::from("/hall/checkout.code-workspace"),
        folders: vec![Utf8PathBuf::from("/hall/.ivar/repos/api/checkout")],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Wrote VSCode workspace for `checkout` to /hall/checkout.code-workspace\n  /hall/.ivar/repos/api/checkout\n"
    );
}
