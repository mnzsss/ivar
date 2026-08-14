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
use crate::test_support::{git, hall_root, seeded_repo};

/// A hall with one seeded repo declared, a feature created, and the repo
/// promoted. Committer identity is set on the bare clone (shared by its
/// worktrees) so `git rebase` — which runs through `git::System`, not the
/// `-c`-flagged test helper — can create its commits on any machine.
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

    git(
        &root.join(".ivar/repos/api/.bare"),
        &["config", "user.name", "ivar tests"],
    );
    git(
        &root.join(".ivar/repos/api/.bare"),
        &["config", "user.email", "tests@ivar.invalid"],
    );

    (guard, root)
}

/// A hall with two branches in the seeded repo — `main`, and `develop`,
/// which carries a commit `main` does not have — a feature created with
/// `base` as given, and the repo promoted onto it. Committer identity is set
/// on the bare clone as in [`hall_with_promoted_feature`].
fn hall_with_promoted_feature_based_on(base: Option<&str>) -> (tempfile::TempDir, Utf8PathBuf) {
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
            base: base.map(str::to_owned),
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

    git(
        &root.join(".ivar/repos/api/.bare"),
        &["config", "user.name", "ivar tests"],
    );
    git(
        &root.join(".ivar/repos/api/.bare"),
        &["config", "user.email", "tests@ivar.invalid"],
    );

    (guard, root)
}

fn rebase_input(name: &str) -> RebaseInput {
    RebaseInput {
        name: name.to_owned(),
        onto: None,
    }
}

/// Commit directly in the default-branch worktree — which advances the
/// shared `main` ref — so the feature branch has something to rebase onto.
fn advance_main(root: &Utf8PathBuf) {
    let worktree = root.join(".ivar/repos/api/main");
    git(
        &worktree,
        &[
            "-c",
            "user.name=ivar tests",
            "-c",
            "user.email=tests@ivar.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "main work",
        ],
    );
}

#[test]
fn rebase_replays_the_feature_work_onto_the_advanced_default_branch() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());

    // Feature work, committed on the feature branch.
    let feature_wt = root.join(".ivar/repos/api/checkout");
    std::fs::write(feature_wt.join("feat.txt"), "feature\n").unwrap();
    git(&feature_wt, &["add", "feat.txt"]);
    git(&feature_wt, &["commit", "-m", "feature work"]);
    // The default branch advances past the branch point.
    advance_main(&root);

    let report = rebase(&ctx, rebase_input("checkout")).unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.repos.len(), 1);
    assert_eq!(report.value.repos[0].status, RebaseStatus::Rebased);
    // The worktree now carries both the feature work and the main work.
    assert!(fs::is_file(&feature_wt.join("feat.txt")).unwrap());
    assert!(
        fs::is_file(&feature_wt.join("README.md")).unwrap(),
        "rebase must leave the base branch's files in place"
    );
}

#[test]
fn rebase_skips_a_dirty_worktree_with_a_warning() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    let feature_wt = root.join(".ivar/repos/api/checkout");
    advance_main(&root);

    // Uncommitted work — untracked files count as dirty.
    std::fs::write(feature_wt.join("notes.md"), "mine\n").unwrap();

    let report = rebase(&ctx, rebase_input("checkout")).unwrap();

    assert_eq!(report.value.repos[0].status, RebaseStatus::Skipped);
    assert!(!report.is_clean());
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.code == "rebase.dirty")
    );
}

#[test]
fn rebase_aborts_on_a_conflict_and_leaves_the_worktree_untouched() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    let feature_wt = root.join(".ivar/repos/api/checkout");
    let main_wt = root.join(".ivar/repos/api/main");

    // Both branches edit the same file, so the replay cannot apply cleanly.
    std::fs::write(feature_wt.join("README.md"), "feature\n").unwrap();
    git(&feature_wt, &["add", "README.md"]);
    git(&feature_wt, &["commit", "-m", "feature edit"]);
    std::fs::write(main_wt.join("README.md"), "main\n").unwrap();
    git(&main_wt, &["add", "README.md"]);
    git(&main_wt, &["commit", "-m", "main edit"]);

    let report = rebase(&ctx, rebase_input("checkout")).unwrap();

    assert_eq!(report.value.repos[0].status, RebaseStatus::Conflicted);
    assert!(!report.is_clean());
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.code == "rebase.conflicted")
    );
    // The abort restored the worktree: no rebase in progress, no unmerged
    // paths, and the branch's own committed content is back.
    let status = std::process::Command::new("git")
        .args(["-C", feature_wt.as_str(), "status", "--porcelain"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&status.stdout), "");
    assert_eq!(
        std::fs::read_to_string(feature_wt.join("README.md")).unwrap(),
        "feature\n"
    );
}

#[test]
fn rebase_is_rejected_for_a_missing_feature() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root);

    let failure = rebase(&ctx, rebase_input("ghost")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.not_found");
}

/// The declared base — not the repo's `default_branch` — is what a rebase
/// replays onto. Work that landed only on `main` must not appear.
#[test]
fn rebase_replays_onto_the_declared_base_not_the_default_branch() {
    let (_guard, root) = hall_with_promoted_feature_based_on(Some("develop"));
    let ctx = Ctx::new(root.clone());
    advance_main(&root);

    let report = rebase(&ctx, rebase_input("checkout")).unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.repos[0].status, RebaseStatus::Rebased);
    let feature_wt = root.join(".ivar/repos/api/checkout");
    assert!(
        fs::is_file(&feature_wt.join("develop-only.txt")).unwrap(),
        "the branch's own develop-derived content must survive"
    );
}

/// `--onto` rewrites every promoted repo's declared base and rebases onto
/// it — the verb for once a feature's own base has landed.
#[test]
fn rebase_onto_collapses_the_base_and_rebases_onto_the_new_target() {
    let (_guard, root) = hall_with_promoted_feature_based_on(Some("develop"));
    let ctx = Ctx::new(root.clone());
    advance_main(&root);

    let report = rebase(
        &ctx,
        RebaseInput {
            name: "checkout".to_owned(),
            onto: Some("main".to_owned()),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.repos[0].status, RebaseStatus::Rebased);
    let feature_wt = root.join(".ivar/repos/api/checkout");
    let history = std::process::Command::new("git")
        .args(["-C", feature_wt.as_str(), "log", "--format=%s"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&history.stdout).contains("main work"),
        "rebased onto `main`, so its commit must be in the branch's history"
    );

    let feature = Feature::read(&Layout::at(root), &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        feature.promotions[&RepoName::new("api").unwrap()].base,
        Some(BranchName::new("main").unwrap())
    );
}

/// A repo `--onto` could not actually rebase keeps its old declared base —
/// recording a target its worktree was never moved onto would leave the
/// next rebase or delivery trusting a base the worktree does not agree with.
#[test]
fn rebase_onto_does_not_collapse_the_base_for_a_repo_it_could_not_rebase() {
    let (_guard, root) = hall_with_promoted_feature_based_on(Some("develop"));
    let ctx = Ctx::new(root.clone());
    advance_main(&root);
    let feature_wt = root.join(".ivar/repos/api/checkout");
    // Uncommitted work — the repo must be skipped, not rebased.
    std::fs::write(feature_wt.join("notes.md"), "mine\n").unwrap();

    let report = rebase(
        &ctx,
        RebaseInput {
            name: "checkout".to_owned(),
            onto: Some("main".to_owned()),
        },
    )
    .unwrap();

    assert_eq!(report.value.repos[0].status, RebaseStatus::Skipped);
    assert!(!report.is_clean());

    let feature = Feature::read(&Layout::at(root), &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        feature.promotions[&RepoName::new("api").unwrap()].base,
        Some(BranchName::new("develop").unwrap()),
        "the declared base must stay `develop` — the worktree was never rebased onto `main`"
    );
}

#[test]
fn the_human_surface_lists_per_repo_status() {
    let outcome = RebaseOutcome {
        root: Utf8PathBuf::from("/hall"),
        feature: FeatureName::new("checkout").unwrap(),
        branch: "checkout".to_owned(),
        repos: vec![RepoRebase {
            repo: RepoName::new("api").unwrap(),
            status: RebaseStatus::Rebased,
        }],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Rebased feature `checkout` (branch: checkout) in /hall:\n  api  rebased\n"
    );
}
