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
            base: None,
            parent: None,
            via: None,
            strategy: None,
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
            recursive: false,
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
            base: None,
        },
    )
    .unwrap();

    let report = status(
        &ctx,
        StatusInput {
            feature: "checkout".to_owned(),
            recursive: false,
        },
    )
    .unwrap();

    let detail = &report.value.repos[0];
    assert_eq!(detail.repo.as_str(), "api");
    assert_eq!(detail.state, WorktreeState::Ready);
    assert!(detail.worktree_present);
    assert_eq!(detail.base, Some(BranchName::new("main").unwrap()));
    assert!(!detail.base_diverged);
}

#[test]
fn status_is_rejected_for_a_missing_feature() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root);

    let failure = status(
        &ctx,
        StatusInput {
            feature: "ghost".to_owned(),
            recursive: false,
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
        is_subfeature: false,
        parent: None,
        plan_approved: false,
        repos: vec![RepoDetail {
            repo: RepoName::new("api").unwrap(),
            state: WorktreeState::Ready,
            worktree_present: true,
            base: Some(BranchName::new("main").unwrap()),
            base_diverged: false,
            pr_url: None,
        }],
        tree: None,
    };
    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Feature `checkout` (branch: checkout) in /hall:\n  api  ready  worktree present  base: main\n"
    );
}

#[test]
fn the_human_surface_marks_a_diverged_base() {
    let outcome = StatusOutcome {
        root: Utf8PathBuf::from("/hall"),
        name: FeatureName::new("checkout").unwrap(),
        branch: "checkout".to_owned(),
        is_subfeature: false,
        parent: None,
        plan_approved: false,
        repos: vec![RepoDetail {
            repo: RepoName::new("api").unwrap(),
            state: WorktreeState::Ready,
            worktree_present: true,
            base: Some(BranchName::new("main").unwrap()),
            base_diverged: true,
            pr_url: None,
        }],
        tree: None,
    };
    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Feature `checkout` (branch: checkout) in /hall:\n  api  ready  worktree present  \
         base: main (diverged from the feature's declared base)\n"
    );
}

/// A hall with two branches in the seeded repo — `main`, and `develop`,
/// which carries a commit `main` does not have — and a feature created with
/// `base` as given. No repo promoted yet.
fn hall_with_two_branches(base: Option<&str>) -> (tempfile::TempDir, Utf8PathBuf) {
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
    crate::test_support::git(&origin, &["checkout", "-b", "develop"]);
    std::fs::write(origin.join("develop-only.txt"), "develop\n").unwrap();
    crate::test_support::git(&origin, &["add", "develop-only.txt"]);
    crate::test_support::git(&origin, &["commit", "-m", "develop work"]);
    crate::test_support::git(&origin, &["checkout", "main"]);

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

    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    (guard, root)
}

/// The feature's declared base, once actually used by `promote`, is what
/// `status` shows — and it agrees with the declaration, so nothing diverges.
#[test]
fn status_shows_the_features_declared_base() {
    let (_guard, root) = hall_with_two_branches(Some("develop"));
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

    let report = status(
        &ctx,
        StatusInput {
            feature: "checkout".to_owned(),
            recursive: false,
        },
    )
    .unwrap();

    let detail = &report.value.repos[0];
    assert_eq!(detail.base, Some(BranchName::new("develop").unwrap()));
    assert!(!detail.base_diverged);
}

/// A per-repo `--base` override at promote time records a base the feature
/// itself did not declare — `status` flags that as diverged, rather than
/// letting the mismatch scroll off screen with the one-time promote warning.
#[test]
fn status_marks_divergence_when_the_recorded_base_disagrees_with_the_declaration() {
    let (_guard, root) = hall_with_two_branches(Some("main"));
    let ctx = Ctx::new(root.clone());
    promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            base: Some("develop".to_owned()),
        },
    )
    .unwrap();

    let report = status(
        &ctx,
        StatusInput {
            feature: "checkout".to_owned(),
            recursive: false,
        },
    )
    .unwrap();

    let detail = &report.value.repos[0];
    assert_eq!(detail.base, Some(BranchName::new("develop").unwrap()));
    assert!(detail.base_diverged);
}

#[test]
fn feature_status_json_surfaces_subfeature_plan_approval_and_pr_url() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let parent = FeatureName::new("checkout").unwrap();
    let child = FeatureName::new("child-feature").unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: child.to_string(),
            branch: None,
            base: None,
            parent: Some(parent.to_string()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    promote::promote(
        &ctx,
        PromoteInput {
            feature: child.to_string(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    let repo = RepoName::new("api").unwrap();
    let mut feature = Feature::read(&layout, &child).unwrap().unwrap();
    let promotion = feature.promotions.get_mut(&repo).unwrap();
    promotion.pr_url = Some("https://github.com/org/repo/pull/42".into());
    feature.write(&layout).unwrap();

    let mut approvals = ApprovalState::fresh();
    approvals.set(Gate::Plan, GateState::Approved, Some("hash123".into()));
    approvals.write(&layout, &child).unwrap();

    let outcome = status(
        &ctx,
        StatusInput {
            feature: child.to_string(),
            recursive: false,
        },
    )
    .expect("status should succeed");

    assert!(outcome.value.is_subfeature);
    assert_eq!(outcome.value.parent, Some(parent));
    assert!(outcome.value.plan_approved);
    assert_eq!(
        outcome.value.repos[0].pr_url.as_deref(),
        Some("https://github.com/org/repo/pull/42")
    );

    let json_val = serde_json::to_value(&outcome.value).unwrap();
    assert_eq!(json_val["is_subfeature"], true);
    assert_eq!(json_val["parent"], "checkout");
    assert_eq!(json_val["plan_approved"], true);
    assert_eq!(
        json_val["repos"][0]["pr_url"],
        "https://github.com/org/repo/pull/42"
    );
}

#[test]
fn feature_status_json_omits_none_parent_and_none_pr_url() {
    let outcome = StatusOutcome {
        root: Utf8PathBuf::from("/hall"),
        name: FeatureName::new("root-feat").unwrap(),
        branch: "root-feat".to_owned(),
        is_subfeature: false,
        parent: None,
        plan_approved: false,
        repos: vec![RepoDetail {
            repo: RepoName::new("api").unwrap(),
            state: WorktreeState::Ready,
            worktree_present: true,
            base: Some(BranchName::new("main").unwrap()),
            base_diverged: false,
            pr_url: None,
        }],
        tree: None,
    };

    let json_val = serde_json::to_value(&outcome).unwrap();
    assert_eq!(json_val["is_subfeature"], false);
    assert!(json_val.get("parent").is_none());
    assert_eq!(json_val["plan_approved"], false);
    assert!(json_val["repos"][0].get("pr_url").is_none());
}

#[test]
fn feature_status_falls_back_to_integration_receipt_pr_url() {
    let (_guard, root) = hall_with_feature();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let child = FeatureName::new("child-feature").unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: child.to_string(),
            branch: None,
            base: None,
            parent: Some("checkout".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    promote::promote(
        &ctx,
        PromoteInput {
            feature: child.to_string(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    let repo = RepoName::new("api").unwrap();
    let mut feature = Feature::read(&layout, &child).unwrap().unwrap();
    let promotion = feature.promotions.get_mut(&repo).unwrap();
    promotion.pr_url = None;
    promotion.integration_receipt = Some(crate::domain::feature::IntegrationReceipt {
        source_sha: "111".to_owned(),
        target_branch: BranchName::new("parent").unwrap(),
        result_sha: "222".to_owned(),
        via: crate::domain::feature::IntegrationVia::Pr,
        strategy: crate::domain::feature::IntegrationStrategy::Squash,
        pr_url: Some("https://github.com/org/repo/pull/99".into()),
        verification: crate::domain::feature::VerificationEvidence {
            command_fingerprint: "checks-v1".to_owned(),
            child: vec![],
            parent: vec![],
            pr_checks: vec![],
            verified_at: crate::domain::session::rfc3339_now(),
        },
    });
    feature.write(&layout).unwrap();

    let outcome = status(
        &ctx,
        StatusInput {
            feature: child.to_string(),
            recursive: false,
        },
    )
    .expect("status should succeed");

    assert_eq!(
        outcome.value.repos[0].pr_url.as_deref(),
        Some("https://github.com/org/repo/pull/99")
    );
}
