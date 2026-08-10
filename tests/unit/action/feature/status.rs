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
use crate::domain::name::{BranchName, HallName};
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

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

    create_action(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
        },
    )
    .unwrap();
    // Materialise the bare clone — promote operates on the cloned repo.
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    (guard, root)
}

#[test]
fn status_shows_a_fresh_feature_with_no_promotions() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root);

    let report = status(
        &ctx,
        StatusInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert!(report.value.repos.is_empty());
    assert_eq!(report.value.branch, "checkout");
}

#[test]
fn status_reports_a_promoted_repo_as_ready_with_its_worktree_present() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root.clone());
    promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
        },
    )
    .unwrap();

    let report = status(
        &ctx,
        StatusInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let detail = &report.value.repos[0];
    assert_eq!(detail.repo.as_str(), "api");
    assert_eq!(detail.state, WorktreeState::Ready);
    assert!(detail.worktree_present);
}

#[test]
fn status_is_rejected_for_a_missing_feature() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root);

    let failure = status(
        &ctx,
        StatusInput {
            feature: "ghost".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.not_found");
}

#[test]
fn the_human_surface_lists_repos_and_their_states() {
    let outcome = StatusOutcome {
        root: Utf8PathBuf::from("/hall"),
        name: FeatureName::new("checkout").unwrap(),
        branch: "checkout".to_owned(),
        repos: vec![RepoDetail {
            repo: RepoName::new("api").unwrap(),
            state: WorktreeState::Ready,
            worktree_present: true,
        }],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Feature `checkout` (branch: checkout) in /hall:\n  api  ready  worktree present\n"
    );
}
