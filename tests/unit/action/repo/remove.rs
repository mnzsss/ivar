#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput};
use crate::action::feature::promote::{self as feature_promote, PromoteInput};
use crate::action::hall::{self, InitInput};
use crate::domain::name::{BranchName, HallName, SessionId};
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::store::manifest::{Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

/// A hall with one seeded repo declared in `ivar.json` — not yet synced.
fn hall_declared() -> (tempfile::TempDir, Utf8PathBuf) {
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

    (guard, root)
}

/// [`hall_declared`] with the repo materialised the way `ivar sync` would.
fn hall_with_repo() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_declared();
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    (guard, root)
}

fn create_feature(ctx: &Ctx, name: &str) {
    feature_create::create(
        ctx,
        CreateInput {
            name: name.to_owned(),
            branch: None,
        },
    )
    .unwrap();
}

fn promote(ctx: &Ctx, feature: &str, repo: &str) {
    feature_promote::promote(
        ctx,
        PromoteInput {
            feature: feature.to_owned(),
            repo: repo.to_owned(),
        },
    )
    .unwrap();
}

fn input(name: &str, force: bool) -> RemoveInput {
    RemoveInput {
        name: name.to_owned(),
        force,
    }
}

fn manifest(root: &Utf8Path) -> Manifest {
    Manifest::read(&Layout::at(root.to_path_buf()))
        .unwrap()
        .unwrap()
}

// -- not in the hall / not in the manifest --------------------------------

#[test]
fn remove_outside_a_hall_is_blocked() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root);

    let failure = remove(&ctx, input("api", false)).unwrap_err();

    assert_eq!(failure.code, "hall.not_found");
}

#[test]
fn remove_rejects_a_repo_that_is_not_in_the_manifest() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root);

    let failure = remove(&ctx, input("ghost", false)).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "manifest.repo_not_found");
}

// -- the gate -------------------------------------------------------------

#[test]
fn remove_refuses_while_the_repo_is_promoted_in_a_feature() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    create_feature(&ctx, "checkout");
    promote(&ctx, "checkout", "api");

    let failure = remove(&ctx, input("api", false)).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "repo.in_use");
    // The blocker is named.
    let actual = failure.actual.as_deref().unwrap();
    assert!(
        actual.contains("checkout"),
        "the blocking feature must be named: {actual}"
    );
    // Nothing was touched.
    assert_eq!(manifest(&root).repos().len(), 1);
    assert!(root.join(".ivar/repos/api/checkout/README.md").is_file());
    assert!(
        !failure.fix_actions[0].safe,
        "removing a promoted repo must need a human"
    );
}

#[test]
fn remove_refuses_while_the_repo_is_referenced_by_a_live_session_view_dir() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    // A live discovery-session view dir referencing the default worktree.
    let session = SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap();
    let view_dir = Layout::at(root.clone()).discovery_session(&session);
    fs::ensure_dir(&view_dir).unwrap();
    fs::create_symlink(&root.join(".ivar/repos/api/main"), &view_dir.join("api")).unwrap();

    let failure = remove(&ctx, input("api", false)).unwrap_err();

    assert_eq!(failure.code, "repo.in_use");
    assert!(
        failure.actual.as_deref().unwrap().contains("view dir"),
        "the blocking view dir must be named: {:?}",
        failure.actual
    );
}

// -- the teardown ---------------------------------------------------------

#[test]
fn remove_without_force_tears_down_a_clean_repo() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());

    let report = remove(&ctx, input("api", false)).unwrap();

    assert!(report.is_clean());
    assert!(manifest(&root).repos().is_empty());
    assert!(
        !fs::exists(&root.join(".ivar/repos/api")).unwrap(),
        "the whole repo tree must go"
    );
    assert!(
        report.value.steps.iter().any(|step| {
            step.label.contains(".ivar/repos/api/") && step.change == Change::Removed
        })
    );
}

#[test]
fn remove_works_for_a_declared_but_never_synced_repo() {
    let (_guard, root) = hall_declared();
    let ctx = Ctx::new(root.clone());

    let report = remove(&ctx, input("api", false)).unwrap();

    assert!(report.is_clean());
    assert!(manifest(&root).repos().is_empty());
}

/// The full cascade: two features promoting the repo, one with a live
/// view dir pointing at its feature worktree.
#[test]
fn remove_force_cascades_across_worktrees_promotions_and_view_dirs() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    create_feature(&ctx, "checkout");
    create_feature(&ctx, "billing");
    promote(&ctx, "checkout", "api");
    promote(&ctx, "billing", "api");
    // A live feature-session view dir referencing the checkout worktree.
    let session = SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap();
    let view_dir =
        Layout::at(root.clone()).feature_session(&FeatureName::new("checkout").unwrap(), &session);
    fs::ensure_dir(&view_dir).unwrap();
    fs::create_symlink(
        &root.join(".ivar/repos/api/checkout"),
        &view_dir.join("api"),
    )
    .unwrap();

    let report = remove(&ctx, input("api", true)).unwrap();

    assert!(report.is_clean());
    // Every worktree and the whole repo tree are gone.
    assert!(!fs::exists(&root.join(".ivar/repos/api")).unwrap());
    // Both features' promotion records are scrubbed.
    for feature in ["checkout", "billing"] {
        let feature = Feature::read(
            &Layout::at(root.clone()),
            &FeatureName::new(feature).unwrap(),
        )
        .unwrap()
        .unwrap();
        assert!(!feature.is_promoted(&RepoName::new("api").unwrap()));
    }
    // The dangling view-dir symlink is unlinked.
    assert_eq!(
        fs::read_symlink(&view_dir.join("api")).unwrap(),
        fs::SymlinkTarget::Absent
    );
    // The manifest no longer lists the repo.
    assert!(manifest(&root).repos().is_empty());
    // The provider config is regenerated: the repo is gone from the block.
    let block = fs::read_text(&root.join("CLAUDE.md")).unwrap().unwrap();
    assert!(
        !block.contains("`api`"),
        "provider config must be regenerated: {block}"
    );
}

/// Best-effort (N-BEST-EFFORT): a teardown step that fails becomes a
/// warning, the manifest is still written (the authoritative step), and a
/// retry can finish whatever is left.
#[test]
fn remove_force_survives_a_failed_step_and_still_writes_the_manifest() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    // A feature whose promotion record exists but whose "worktree" is a
    // hand-made directory git does not recognise — `git worktree remove`
    // refuses it, exercising the best-effort path.
    create_feature(&ctx, "checkout");
    let mut feature = Feature::read(
        &Layout::at(root.clone()),
        &FeatureName::new("checkout").unwrap(),
    )
    .unwrap()
    .unwrap();
    feature.promote(RepoName::new("api").unwrap());
    feature.write(&Layout::at(root.clone())).unwrap();
    let stray = root.join(".ivar/repos/api/checkout");
    fs::ensure_dir(&stray).unwrap();
    fs::write_text(&stray.join("mine.txt"), "mine").unwrap();

    let report = remove(&ctx, input("api", true)).unwrap();

    assert!(!report.is_clean(), "a failed step must not be a clean run");
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.code == "repo.remove_step_failed"),
        "the failed step must surface as a warning"
    );
    // The manifest write is authoritative: the repo is gone from ivar.json.
    assert!(manifest(&root).repos().is_empty());
    // And the whole repo tree went anyway.
    assert!(!fs::exists(&root.join(".ivar/repos/api")).unwrap());
}

#[test]
fn the_human_surface_lists_the_teardown_steps() {
    let outcome = RemoveOutcome {
        root: Utf8PathBuf::from("/hall"),
        name: RepoName::new("api").unwrap(),
        steps: vec![
            Entry::new("feature checkout", "worktree checkout", Change::Removed),
            Entry::new("hall", "ivar.json", Change::Updated),
        ],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Removed repo `api` from /hall\n  - worktree checkout\n  ~ ivar.json\n"
    );
}
