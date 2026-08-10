#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::feature::create::{CreateInput, create as feature_create};
use crate::action::feature::promote::{PromoteInput, promote as feature_promote};
use crate::action::hall::{self, InitInput};
use crate::domain::name::{BranchName, HallName};
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

/// A hall with one seeded repo declared, and a feature created.
fn hall_with_feature() -> (tempfile::TempDir, Utf8PathBuf) {
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

    let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            origin.as_str(),
            BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    feature_create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
        },
    )
    .unwrap();
    // Materialise the bare clone, the way `ivar sync` would.
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    (guard, root)
}

#[test]
fn view_reports_the_promoted_repos_and_worktrees_without_a_tty() {
    // Tests run without a terminal, so `view` skips the TUI and reports.
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root.clone());
    feature_promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
        },
    )
    .unwrap();

    let report = view(
        &ctx,
        ViewInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.feature.as_str(), "checkout");
    assert_eq!(report.value.branch, "checkout");
    assert_eq!(report.value.repos.len(), 1);
    assert_eq!(report.value.repos[0].as_str(), "api");
}

#[test]
fn view_collects_one_shell_per_promoted_repo_in_worktree_order() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root.clone());
    feature_promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
        },
    )
    .unwrap();

    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let repos: Vec<RepoName> = feature.promotions.keys().cloned().collect();
    let worktree = layout.repo_worktree(&repos[0], &feature.branch);

    assert!(
        worktree.as_str().contains(".ivar/repos/api/checkout"),
        "the shell runs in the repo's feature worktree: {worktree}"
    );
    assert!(fs::is_dir(&worktree).unwrap(), "the worktree exists");
}

#[test]
fn view_is_rejected_for_a_missing_feature() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root);

    let failure = view(
        &ctx,
        ViewInput {
            feature: "ghost".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.not_found");
}

#[test]
fn view_is_rejected_for_a_feature_with_no_promotions() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root);

    let failure = view(
        &ctx,
        ViewInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.view_no_repos");
}

#[test]
fn the_human_surface_lists_the_repos_and_shell_count() {
    let outcome = ViewOutcome {
        root: Utf8PathBuf::from("/hall"),
        feature: FeatureName::new("checkout").unwrap(),
        branch: "checkout".to_owned(),
        repos: vec![RepoName::new("api").unwrap(), RepoName::new("web").unwrap()],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Feature `checkout` (branch: checkout) in /hall:\n  api\n  web\n2 shells opened\n"
    );
}

#[test]
fn user_shell_falls_back_to_bash_when_unset() {
    assert_eq!(resolve_shell(None), "bash");
    assert_eq!(resolve_shell(Some("fish")), "fish");
}
