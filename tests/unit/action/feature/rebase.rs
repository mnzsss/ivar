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
        &InitInput {
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
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
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
        &InitInput {
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
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
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

/// Default input: remote-first, every promoted repo.
fn rebase_input(name: &str) -> RebaseInput {
    RebaseInput {
        name: name.to_owned(),
        onto: None,
        repos: Vec::new(),
        offline: false,
    }
}

/// `--offline`: the local base ref, no network call. For tests that
/// advance the hall's *local* `main` — the origin did not move, so a
/// remote-first rebase would (correctly) not replay those commits.
fn offline_input(name: &str) -> RebaseInput {
    RebaseInput {
        offline: true,
        ..rebase_input(name)
    }
}

/// The origin repo the seeded hall cloned `api` from.
fn origin_of(root: &Utf8PathBuf, repo: &str) -> Utf8PathBuf {
    root.parent().unwrap().join("origins").join(repo)
}

/// A commit on the origin's `main` the hall has not fetched — a PR merged
/// on the remote after the last `ivar sync` (issue #156).
fn advance_origin_main(root: &Utf8PathBuf) {
    git(
        &origin_of(root, "api"),
        &["commit", "--allow-empty", "-m", "origin work"],
    );
}

fn commit_feature_work(root: &Utf8PathBuf) -> Utf8PathBuf {
    let feature_wt = root.join(".ivar/repos/api/checkout");
    std::fs::write(feature_wt.join("feat.txt"), "feature\n").unwrap();
    git(&feature_wt, &["add", "feat.txt"]);
    git(&feature_wt, &["commit", "-m", "feature work"]);
    feature_wt
}

fn log_subjects(worktree: &Utf8PathBuf) -> String {
    let out = std::process::Command::new("git")
        .args(["-C", worktree.as_str(), "log", "--format=%s"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn head_of(worktree: &Utf8PathBuf) -> String {
    let out = std::process::Command::new("git")
        .args(["-C", worktree.as_str(), "rev-parse", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A hall with `repos` declared (each seeded on `main`), feature
/// `checkout` created, and every repo promoted onto it.
fn hall_with_promoted_repos(repos: &[&str]) -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();

    let declared = repos
        .iter()
        .map(|name| {
            let origin = seeded_repo(&origin_of(&root, name), "main");
            Repo::new(
                RepoName::new(*name).unwrap(),
                origin.as_str(),
                BranchName::new("main").unwrap(),
            )
        })
        .collect();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        declared,
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
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    for name in repos {
        promote::promote(
            &ctx,
            PromoteInput {
                feature: "checkout".to_owned(),
                repo: (*name).to_owned(),
                base: None,
            },
        )
        .unwrap();
        let bare = root.join(format!(".ivar/repos/{name}/.bare"));
        git(&bare, &["config", "user.name", "ivar tests"]);
        git(&bare, &["config", "user.email", "tests@ivar.invalid"]);
    }
    (guard, root)
}

/// Attach a passing integration receipt to `repo`'s promotion, which makes
/// `ensure_promotion_mutable` refuse it.
fn lock_promotion(root: &Utf8PathBuf, repo: &str) {
    let layout = Layout::at(root.clone());
    let mut feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    feature
        .promotions
        .get_mut(&RepoName::new(repo).unwrap())
        .unwrap()
        .integration_receipt = Some(crate::domain::feature::IntegrationReceipt {
        source_sha: "111".to_owned(),
        target_branch: BranchName::new("main").unwrap(),
        result_sha: "222".to_owned(),
        via: crate::domain::feature::IntegrationVia::Local,
        strategy: crate::domain::feature::IntegrationStrategy::Squash,
        pr_url: None,
        verification: crate::domain::feature::VerificationEvidence {
            command_fingerprint: "checks-v1".to_owned(),
            child: Vec::new(),
            parent: Vec::new(),
            pr_checks: Vec::new(),
            verified_at: "2026-10-06T12:00:00Z".to_owned(),
        },
    });
    feature.write(&layout).unwrap();
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

    let report = rebase(&ctx, offline_input("checkout")).unwrap();

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

    let report = rebase(&ctx, offline_input("checkout")).unwrap();

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
            repos: Vec::new(),
            offline: true,
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
            repos: Vec::new(),
            offline: true,
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
fn the_human_surface_lists_per_repo_status_and_base() {
    let outcome = RebaseOutcome {
        root: Utf8PathBuf::from("/hall"),
        feature: FeatureName::new("checkout").unwrap(),
        branch: "checkout".to_owned(),
        repos: vec![
            RepoRebase {
                repo: RepoName::new("api").unwrap(),
                status: RebaseStatus::Rebased,
                onto: Some(BranchName::new("main").unwrap()),
                base_source: Some(BaseSource::Remote),
            },
            RepoRebase {
                repo: RepoName::new("cli").unwrap(),
                status: RebaseStatus::Conflicted,
                onto: Some(BranchName::new("develop").unwrap()),
                base_source: Some(BaseSource::Local),
            },
            RepoRebase {
                repo: RepoName::new("web").unwrap(),
                status: RebaseStatus::Skipped,
                onto: Some(BranchName::new("main").unwrap()),
                base_source: None,
            },
        ],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Rebased feature `checkout` (branch: checkout) in /hall:\n\
         \x20 api  rebased  onto main (remote)\n\
         \x20 cli  conflicted  onto develop (local)\n\
         \x20 web  skipped\n"
    );
}

#[test]
fn the_json_surface_carries_onto_and_base_source() {
    let repo = RepoRebase {
        repo: RepoName::new("api").unwrap(),
        status: RebaseStatus::Rebased,
        onto: Some(BranchName::new("main").unwrap()),
        base_source: Some(BaseSource::Remote),
    };
    let json = serde_json::to_value(&repo).unwrap();
    assert_eq!(json["onto"], "main");
    assert_eq!(json["base_source"], "remote");
}

/// Issue #156: the hall's local `main` is behind the remote. The rebase
/// lands on the remote tip — the one `deliver` checks — and leaves the
/// local `main` alone.
#[test]
fn rebase_replays_onto_the_remote_tip_when_the_local_base_is_behind() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    let feature_wt = commit_feature_work(&root);
    advance_origin_main(&root);

    let report = rebase(&ctx, rebase_input("checkout")).unwrap();

    assert!(report.is_clean(), "{:?}", report.warnings);
    let repo = &report.value.repos[0];
    assert_eq!(repo.status, RebaseStatus::Rebased);
    assert_eq!(repo.onto, Some(BranchName::new("main").unwrap()));
    assert_eq!(repo.base_source, Some(BaseSource::Remote));
    let history = log_subjects(&feature_wt);
    assert!(history.contains("origin work"), "{history}");
    assert!(history.contains("feature work"), "{history}");
    assert!(
        !log_subjects(&root.join(".ivar/repos/api/main")).contains("origin work"),
        "rebase must not move the local default branch"
    );
}

/// A base the remote does not carry — an unpublished parent branch — is
/// rebased onto locally, silently: that is the expected case.
#[test]
fn rebase_uses_the_local_base_without_a_warning_when_the_remote_lacks_it() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    let feature_wt = commit_feature_work(&root);
    advance_main(&root);
    git(
        &root.join(".ivar/repos/api/.bare"),
        &["branch", "local-only", "main"],
    );

    let report = rebase(
        &ctx,
        RebaseInput {
            onto: Some("local-only".to_owned()),
            ..rebase_input("checkout")
        },
    )
    .unwrap();

    assert!(report.is_clean(), "{:?}", report.warnings);
    let repo = &report.value.repos[0];
    assert_eq!(repo.status, RebaseStatus::Rebased);
    assert_eq!(repo.onto, Some(BranchName::new("local-only").unwrap()));
    assert_eq!(repo.base_source, Some(BaseSource::Local));
    assert!(log_subjects(&feature_wt).contains("main work"));
}

/// The remote does not answer: the local base is used, and the report
/// says so, so a stale base is never silent.
#[test]
fn rebase_falls_back_to_the_local_base_with_a_warning_when_the_remote_is_unreachable() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    let feature_wt = commit_feature_work(&root);
    advance_main(&root);
    let origin = origin_of(&root, "api");
    std::fs::rename(&origin, origin.with_extension("gone")).unwrap();

    let report = rebase(&ctx, rebase_input("checkout")).unwrap();

    let repo = &report.value.repos[0];
    assert_eq!(repo.status, RebaseStatus::Rebased);
    assert_eq!(repo.base_source, Some(BaseSource::Local));
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.code == "rebase.remote_unreachable" && w.subject == "api"),
        "{:?}",
        report.warnings
    );
    assert!(log_subjects(&feature_wt).contains("main work"));
}

/// `--offline` never asks the remote: an unreachable origin raises no
/// warning, and a remote that moved ahead is not followed.
#[test]
fn rebase_offline_makes_no_network_call_and_ignores_the_remote() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    let feature_wt = commit_feature_work(&root);
    advance_origin_main(&root);
    let origin = origin_of(&root, "api");
    std::fs::rename(&origin, origin.with_extension("gone")).unwrap();

    let report = rebase(&ctx, offline_input("checkout")).unwrap();

    assert!(report.is_clean(), "{:?}", report.warnings);
    assert_eq!(report.value.repos[0].base_source, Some(BaseSource::Local));
    assert!(!log_subjects(&feature_wt).contains("origin work"));
}

/// `--repo` limits the preflight and the batch: a locked, unselected repo
/// no longer blocks, and only the selected repo is reported.
#[test]
fn rebase_with_repo_only_checks_and_touches_the_selected_repos() {
    let (_guard, root) = hall_with_promoted_repos(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    lock_promotion(&root, "web");

    let blocked = rebase(&ctx, offline_input("checkout")).unwrap_err();
    assert_eq!(blocked.code, "feature.promotion_integration_immutable");

    let report = rebase(
        &ctx,
        RebaseInput {
            repos: vec!["api".to_owned(), "api".to_owned()],
            ..offline_input("checkout")
        },
    )
    .unwrap();

    assert!(report.is_clean(), "{:?}", report.warnings);
    assert_eq!(report.value.repos.len(), 1, "duplicates collapse to one");
    assert_eq!(report.value.repos[0].repo, RepoName::new("api").unwrap());
}

/// A `--repo` that is not promoted is refused before anything moves.
#[test]
fn rebase_rejects_a_repo_that_is_not_promoted_before_touching_anything() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    let feature_wt = commit_feature_work(&root);
    advance_origin_main(&root);
    let before = head_of(&feature_wt);

    let failure = rebase(
        &ctx,
        RebaseInput {
            repos: vec!["api".to_owned(), "ghost".to_owned()],
            ..rebase_input("checkout")
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.not_promoted");
    assert!(failure.what.contains("ghost"));
    assert_eq!(head_of(&feature_wt), before, "no repo may be rebased");
}
