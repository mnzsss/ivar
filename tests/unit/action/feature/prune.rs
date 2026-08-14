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
use crate::domain::name::{BranchName, HallName, RepoName, SessionId};
use crate::domain::provider::Provider;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{git, hall_root, seeded_repo};

/// A hall with one synced repo and one promoted feature (`checkout`) —
/// its branch is off `main` with no new commits, so it is immediately
/// merged.
fn hall_with_promoted_feature() -> (tempfile::TempDir, Utf8PathBuf) {
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

/// A hall with two branches — `main`, and `develop`, which carries a commit
/// `main` does not have (standing in for an undelivered parent feature) —
/// and `checkout` promoted onto `develop` as its declared base.
fn hall_with_promoted_feature_based_on_an_open_base() -> (tempfile::TempDir, Utf8PathBuf) {
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
    git(&origin, &["checkout", "-b", "develop"]);
    std::fs::write(origin.join("develop-only.txt"), "develop\n").unwrap();
    git(&origin, &["add", "develop-only.txt"]);
    git(&origin, &["commit", "-m", "develop work"]);
    git(&origin, &["checkout", "main"]);

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
            base: Some("develop".to_owned()),
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

fn feature_dir(root: &Utf8PathBuf) -> Utf8PathBuf {
    root.join(".ivar/features/checkout")
}

#[test]
fn prune_deletes_a_feature_whose_branch_is_merged() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());

    let report = prune(&ctx).unwrap();

    assert!(report.is_clean());
    assert_eq!(
        report.value.pruned,
        vec![FeatureName::new("checkout").unwrap()]
    );
    assert!(report.value.kept.is_empty());
    assert!(
        !feature_dir(&root).exists(),
        "the feature directory is gone"
    );
    assert!(
        !root.join(".ivar/repos/api/checkout").exists(),
        "the worktree is gone"
    );
}

/// The hard guard: a feature with a live session is never touched, no
/// matter how merged its branches are.
#[test]
fn prune_never_touches_a_feature_with_a_live_session() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());

    // A live session: its view dir exists under the feature's sessions.
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let session = SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap();
    fs::ensure_dir(&layout.feature_session(&feature, &session)).unwrap();

    let report = prune(&ctx).unwrap();

    assert!(report.is_clean());
    assert!(report.value.pruned.is_empty());
    assert_eq!(report.value.kept.len(), 1);
    assert_eq!(report.value.kept[0].feature.as_str(), "checkout");
    assert!(report.value.kept[0].reason.contains("live session"));
    assert!(
        feature_dir(&root).join("feature.json").exists(),
        "a live-session feature must be left fully intact"
    );
    assert!(root.join(".ivar/repos/api/checkout").exists());
}

#[test]
fn prune_keeps_a_feature_with_unmerged_commits() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());

    // A commit on the feature branch that `main` does not have.
    let worktree = root.join(".ivar/repos/api/checkout");
    std::fs::write(worktree.join("work.md"), "work\n").unwrap();
    git(&worktree, &["add", "work.md"]);
    git(&worktree, &["commit", "-m", "work"]);

    let report = prune(&ctx).unwrap();

    assert!(report.is_clean());
    assert!(report.value.pruned.is_empty());
    assert_eq!(report.value.kept.len(), 1);
    assert!(
        report.value.kept[0].reason.contains("not merged"),
        "reason was: {}",
        report.value.kept[0].reason
    );
    assert!(feature_dir(&root).join("feature.json").exists());
}

#[test]
fn prune_keeps_a_feature_whose_clone_is_missing() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    // Delete the bare clone behind ivar's back — merge can no longer be
    // judged, so the feature must be kept, not pruned on a guess.
    fs::remove_path(&root.join(".ivar/repos/api/.bare")).unwrap();

    let report = prune(&ctx).unwrap();

    assert!(report.is_clean());
    assert!(report.value.pruned.is_empty());
    assert_eq!(report.value.kept.len(), 1);
    assert!(
        report.value.kept[0].reason.contains("cannot check"),
        "reason was: {}",
        report.value.kept[0].reason
    );
    assert!(feature_dir(&root).join("feature.json").exists());
}

/// Mergedness is measured against what the feature actually branched from —
/// so a feature merged into its declared base is prunable even while that
/// base itself is still open (not merged into the repo's default branch).
#[test]
fn prune_deletes_a_feature_merged_into_its_still_open_base() {
    let (_guard, root) = hall_with_promoted_feature_based_on_an_open_base();
    let ctx = Ctx::new(root.clone());

    let report = prune(&ctx).unwrap();

    assert!(report.is_clean());
    assert_eq!(
        report.value.pruned,
        vec![FeatureName::new("checkout").unwrap()]
    );
    assert!(report.value.kept.is_empty());
}

#[test]
fn the_human_surface_names_pruned_and_kept() {
    let outcome = PruneOutcome {
        root: Utf8PathBuf::from("/hall"),
        pruned: vec![FeatureName::new("checkout").unwrap()],
        kept: vec![KeptFeature {
            feature: FeatureName::new("checkout").unwrap(),
            reason: "has a live session".to_owned(),
        }],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Pruned feature `checkout`.\nKept `checkout` — has a live session.\n"
    );
}
