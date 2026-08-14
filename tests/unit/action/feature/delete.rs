#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::os::unix::fs::PermissionsExt as _;

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

/// A hall with one seeded repo declared, a feature created, and the repo
/// promoted (so a real worktree exists to tear down).
fn hall_with_promoted_feature() -> (tempfile::TempDir, Utf8PathBuf) {
    hall_with_promoted_feature_on(None)
}

/// The same hall, with the feature on an explicit branch. A branch holding a
/// `/` — `feat/checkout` — nests the worktree one directory deeper, which is
/// what the orphan-parent teardown has to cope with.
fn hall_with_promoted_feature_on(branch: Option<&str>) -> (tempfile::TempDir, Utf8PathBuf) {
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
            branch: branch.map(str::to_owned),
            base: None,
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

fn delete_input(name: &str) -> DeleteInput {
    DeleteInput {
        name: name.to_owned(),
    }
}

#[test]
fn delete_removes_worktrees_the_feature_dir_and_plans() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    // A plan artifact to tear down alongside.
    fs::ensure_dir(&root.join("plans/checkout")).unwrap();
    fs::write_text(&root.join("plans/checkout/plan.md"), "# Plan\n").unwrap();

    let report = delete(&ctx, delete_input("checkout")).unwrap();

    assert!(report.is_clean());
    assert!(report.value.feature_removed);
    assert!(report.value.plans_removed);
    assert_eq!(report.value.worktrees.len(), 1);
    assert!(report.value.worktrees[0].removed);

    assert!(!fs::exists(&root.join(".ivar/features/checkout")).unwrap());
    assert!(!fs::exists(&root.join("plans/checkout")).unwrap());
    assert!(!fs::exists(&root.join(".ivar/repos/api/checkout")).unwrap());
}

#[test]
fn delete_leaves_no_empty_parent_behind_a_slashed_branch() {
    let (_guard, root) = hall_with_promoted_feature_on(Some("feat/checkout"));
    let ctx = Ctx::new(root.clone());
    assert!(fs::is_dir(&root.join(".ivar/repos/api/feat/checkout")).unwrap());

    let report = delete(&ctx, delete_input("checkout")).unwrap();

    assert!(report.is_clean());
    assert!(report.value.worktrees[0].removed);
    assert!(!fs::exists(&root.join(".ivar/repos/api/feat/checkout")).unwrap());
    // `feat/` only ever existed to hold that worktree: it goes with it.
    assert!(
        !fs::exists(&root.join(".ivar/repos/api/feat")).unwrap(),
        "the branch prefix directory was left behind as an empty orphan"
    );
    // The repo dir itself is the floor — pruning stops there.
    assert!(fs::is_dir(&root.join(".ivar/repos/api")).unwrap());
    assert!(fs::is_dir(&root.join(".ivar/repos/api/.bare")).unwrap());
}

#[test]
fn delete_preflight_blocks_on_an_unwritable_path_and_mutates_nothing() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());

    // A directory with its write bits stripped — the preflight must name
    // it and refuse, leaving the feature fully intact.
    let planning = root.join(".ivar/features/checkout/planning");
    fs::ensure_dir(&planning).unwrap();
    fs::write_text(&planning.join("approvals.json"), "{}").unwrap();
    let original = fs_err::metadata(planning.as_std_path())
        .unwrap()
        .permissions()
        .mode();
    fs_err::set_permissions(
        planning.as_std_path(),
        std::fs::Permissions::from_mode(0o555),
    )
    .unwrap();

    let failure = delete(&ctx, delete_input("checkout")).unwrap_err();
    // Restore so TempDir can clean up.
    fs_err::set_permissions(
        planning.as_std_path(),
        std::fs::Permissions::from_mode(original),
    )
    .unwrap();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.delete_blocked");
    let blockers: Vec<DeleteBlocker> =
        serde_json::from_value(failure.details.clone().expect("blockers in details")).unwrap();
    assert!(
        blockers.iter().any(|blocker| blocker.path == planning),
        "blockers were: {blockers:?}"
    );
    assert_eq!(blockers[0].mode.unwrap() & 0o222, 0);
    // Nothing was mutated.
    assert!(fs::is_file(&root.join(".ivar/features/checkout/feature.json")).unwrap());
    assert!(fs::is_dir(&root.join(".ivar/repos/api/checkout")).unwrap());
}

#[test]
fn delete_after_a_successful_delete_is_a_clean_refusal() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    delete(&ctx, delete_input("checkout")).unwrap();

    // The record is gone, so a second delete refuses cleanly — the system
    // is in a stable, fully-deleted state, and retrying is safe.
    let failure = delete(&ctx, delete_input("checkout")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.not_found");
}

#[test]
fn delete_is_rejected_for_a_missing_feature() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root);

    let failure = delete(&ctx, delete_input("ghost")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.not_found");
}

#[test]
fn the_human_surface_names_what_was_deleted() {
    let outcome = DeleteOutcome {
        root: Utf8PathBuf::from("/hall"),
        name: FeatureName::new("checkout").unwrap(),
        worktrees: vec![WorktreeRemoval {
            repo: RepoName::new("api").unwrap(),
            removed: true,
            detail: None,
        }],
        feature_removed: true,
        plans_removed: true,
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Deleted feature `checkout` in /hall\n"
    );
}
