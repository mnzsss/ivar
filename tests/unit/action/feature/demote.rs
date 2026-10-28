#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use rstest::rstest;

use super::*;
use crate::action::feature::create::CreateInput;
use crate::action::feature::create::create as create_action;
use crate::action::feature::promote::{self, PromoteInput};
use crate::action::hall::{self, InitInput};
use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, HallName};
use crate::domain::provider::Provider;
use crate::error::{Outcome, Status};
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

/// A hall with one seeded repo declared and a feature created.
fn hall_with_feature() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
            git: false,
            origin: None,
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

    // Materialise the bare clone — promote operates on the cloned repo.
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();

    (guard, root)
}

#[test]
fn demote_removes_the_promotion_record_and_keeps_the_worktree() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root.clone());
    promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    let report = demote(
        &ctx,
        DemoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            remove_worktree: false,
        },
    )
    .unwrap();

    assert!(report.is_clean());
    let feature = Feature::read(
        &Layout::at(root.clone()),
        &FeatureName::new("checkout").unwrap(),
    )
    .unwrap()
    .unwrap();
    assert!(!feature.is_promoted(&RepoName::new("api").unwrap()));
    // The worktree stays.
    assert!(root.join(".ivar/repos/api/checkout/README.md").is_file());
}

#[rstest]
#[case::the_feature_does_not_exist("ghost", "feature.not_found")]
#[case::the_repo_was_never_promoted("checkout", "feature.not_promoted")]
fn demote_is_rejected_when(#[case] feature: &str, #[case] code: &str) {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root);

    let failure = demote(
        &ctx,
        DemoteInput {
            feature: feature.to_owned(),
            repo: "api".to_owned(),
            remove_worktree: false,
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, code);
}

/// A promotion with a successful integration receipt is individually
/// immutable: demoting it refuses before any write.
#[test]
fn demote_refuses_a_promotion_locked_by_a_successful_receipt() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root.clone());
    promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    // Attach a passing receipt directly — the integrate action lands later.
    let layout = Layout::at(root.clone());
    let mut feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let receipt = crate::domain::feature::IntegrationReceipt {
        source_sha: "111".to_owned(),
        target_branch: BranchName::new("parent").unwrap(),
        result_sha: "222".to_owned(),
        via: crate::domain::feature::IntegrationVia::Local,
        strategy: crate::domain::feature::IntegrationStrategy::Squash,
        pr_url: None,
        verification: crate::domain::feature::VerificationEvidence {
            command_fingerprint: "checks-v1".to_owned(),
            child: Vec::new(),
            parent: Vec::new(),
            pr_checks: Vec::new(),
            verified_at: "2026-08-14T12:00:00Z".to_owned(),
        },
    };
    feature
        .promotions
        .get_mut(&RepoName::new("api").unwrap())
        .unwrap()
        .integration_receipt = Some(receipt);
    feature.write(&layout).unwrap();

    let failure = demote(
        &ctx,
        DemoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            remove_worktree: false,
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.promotion_integration_immutable");

    // The record is untouched — the refusal happened before any write.
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    assert!(feature.is_promoted(&RepoName::new("api").unwrap()));
    assert!(
        feature
            .promotions
            .get(&RepoName::new("api").unwrap())
            .unwrap()
            .integration_receipt
            .is_some()
    );
}

#[test]
fn the_human_surface_says_the_worktree_stays() {
    let outcome = DemoteOutcome {
        root: Utf8PathBuf::from("/hall"),
        feature: FeatureName::new("checkout").unwrap(),
        repo: RepoName::new("api").unwrap(),
        worktree_removed: false,
        branch_deleted: false,
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Demoted `api` from feature `checkout`. Its worktree stays on disk; remove it with `ivar feature demote checkout api --remove-worktree`.\n"
    );
}

fn promoted_checkout() -> (tempfile::TempDir, Utf8PathBuf, Ctx) {
    let (guard, root) = hall_with_feature();
    let ctx = Ctx::new(root.clone());
    promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();
    (guard, root, ctx)
}

fn demote_removing_worktree(ctx: &Ctx) -> Outcome<DemoteOutcome> {
    demote(
        ctx,
        DemoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            remove_worktree: true,
        },
    )
}

fn git_in(dir: &Utf8PathBuf, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-C",
            dir.as_str(),
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
}

fn branch_exists(root: &Utf8PathBuf, branch: &str) -> bool {
    std::process::Command::new("git")
        .args(["--git-dir", root.join(".ivar/repos/api/.bare").as_str()])
        .args(["rev-parse", "--verify", "--quiet", branch])
        .output()
        .unwrap()
        .status
        .success()
}

#[test]
fn demote_with_remove_worktree_drops_a_clean_unpublished_worktree_and_its_branch() {
    let (_guard, root, ctx) = promoted_checkout();

    let report = demote_removing_worktree(&ctx).unwrap();

    assert!(report.is_clean());
    assert!(report.value.worktree_removed);
    assert!(report.value.branch_deleted);
    assert!(!root.join(".ivar/repos/api/checkout").exists());
    assert!(!branch_exists(&root, "checkout"));
    assert!(branch_exists(&root, "main"));
    assert!(root.join(".ivar/repos/api/main").exists());
}

#[rstest]
#[case::uncommitted_changes("dirty", "uncommitted or untracked")]
#[case::commits_beyond_the_base("ahead", "beyond `main`")]
#[case::a_published_branch("published", "published on the remote")]
fn demote_with_remove_worktree_refuses_and_removes_nothing_with(
    #[case] state: &str,
    #[case] blocker: &str,
) {
    let (_guard, root, ctx) = promoted_checkout();
    let worktree = root.join(".ivar/repos/api/checkout");
    match state {
        "dirty" => std::fs::write(worktree.join("scratch.txt"), "wip\n").unwrap(),
        "ahead" => {
            std::fs::write(worktree.join("feat.txt"), "feature\n").unwrap();
            git_in(&worktree, &["add", "feat.txt"]);
            git_in(&worktree, &["commit", "-m", "feature work"]);
        }
        _ => {
            let origin = root.parent().unwrap().join("origins").join("api");
            git_in(
                &worktree,
                &["push", origin.as_str(), "checkout:refs/heads/checkout"],
            );
        }
    }

    let failure = demote_removing_worktree(&ctx).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.demote_worktree_not_removable");
    assert!(failure.actual.unwrap().contains(blocker));
    assert!(worktree.exists());
    assert!(branch_exists(&root, "checkout"));
    let feature = Feature::read(
        &Layout::at(root.clone()),
        &FeatureName::new("checkout").unwrap(),
    )
    .unwrap()
    .unwrap();
    assert!(feature.is_promoted(&RepoName::new("api").unwrap()));
}

#[test]
fn demote_with_remove_worktree_finishes_a_plain_demote_left_on_disk() {
    let (_guard, root, ctx) = promoted_checkout();
    demote(
        &ctx,
        DemoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            remove_worktree: false,
        },
    )
    .unwrap();

    let report = demote_removing_worktree(&ctx).unwrap();

    assert!(report.value.worktree_removed);
    assert!(report.value.branch_deleted);
    assert!(!root.join(".ivar/repos/api/checkout").exists());
    assert!(!branch_exists(&root, "checkout"));
}

#[test]
fn demote_with_remove_worktree_still_reports_a_repo_never_promoted() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root);

    let failure = demote_removing_worktree(&ctx).unwrap_err();

    assert_eq!(failure.code, "feature.not_promoted");
}
