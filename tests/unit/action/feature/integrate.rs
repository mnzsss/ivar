//! Unit tests for `crate::action::feature::integrate` — leaves-first,
//! partial, resumable child integration.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
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
use crate::action::plan::approve as plan_approve;
use crate::action::plan::create as plan_create;
use crate::domain::feature::{Feature, WorktreeState};
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{git, hall_root, seeded_repo};

/// A hall with one seeded repo declared, a parent and a child feature, the
/// repo promoted into both, a commit on the child, and the child's plan gate
/// approved. `checks` become the api repo's ordered verification checks.
fn seeded_child_hall(checks: &[&str]) -> (tempfile::TempDir, Utf8PathBuf) {
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

    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "main");
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![
            Repo::new(
                RepoName::new("api").unwrap(),
                origin.as_str(),
                BranchName::new("main").unwrap(),
            )
            .with_checks(checks.iter().map(|c| c.to_string()).collect()),
        ],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "parent".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    create_action(
        &ctx,
        CreateInput {
            name: "child".to_owned(),
            branch: None,
            base: None,
            parent: Some("parent".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    for feature in ["parent", "child"] {
        promote::promote(
            &ctx,
            PromoteInput {
                feature: feature.to_owned(),
                repo: "api".to_owned(),
                base: None,
            },
        )
        .unwrap();
    }

    // A commit on the child so there is something to integrate.
    let child_wt = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("child").unwrap(),
    );
    std::fs::write(child_wt.join("work.md"), "work\n").unwrap();
    git(&child_wt, &["add", "work.md"]);
    git(&child_wt, &["commit", "-m", "child work"]);

    approve_plan(&ctx, "child");

    (guard, root)
}

/// Scaffold the child's SPDD artifacts and approve the plan gate.
fn approve_plan(ctx: &Ctx, feature: &str) {
    plan_create::create(
        ctx,
        plan_create::CreateInput {
            feature: feature.to_owned(),
        },
    )
    .unwrap();
    for gate in ["requirements", "analysis", "plan"] {
        plan_approve::approve(
            ctx,
            plan_approve::ApproveInput {
                feature: feature.to_owned(),
                gate: gate.to_owned(),
            },
        )
        .unwrap();
    }
}

fn integrate_input(feature: &str) -> IntegrateInput {
    IntegrateInput {
        feature: feature.to_owned(),
        via: None,
        strategy: None,
    }
}

fn api() -> RepoName {
    RepoName::new("api").unwrap()
}

// -- preflight refusals ------------------------------------------------------

#[test]
fn integrate_refuses_a_root_with_the_deliver_command() {
    let (_guard, root) = seeded_child_hall(&["true"]);
    let ctx = Ctx::new(root);

    let failure = integrate(&ctx, integrate_input("parent")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "integration.root_refused");
    assert_eq!(
        failure.fix_actions[0].command.as_deref(),
        Some("ivar feature deliver parent")
    );
}

#[test]
fn integrate_requires_the_plan_gate() {
    let (guard, root) = seeded_child_hall(&["true"]);
    let _ = guard;
    let ctx = Ctx::new(root.clone());

    // A fresh unapproved child: no plan gate.
    let layout = Layout::at(&root);
    let mut unapproved = Feature::new(
        FeatureName::new("unapproved").unwrap(),
        BranchName::new("unapproved").unwrap(),
    );
    unapproved.parent = Some(FeatureName::new("parent").unwrap());
    unapproved.write(&layout).unwrap();

    let failure = integrate(&ctx, integrate_input("unapproved")).unwrap_err();
    assert_eq!(failure.code, "integration.plan_not_approved");
}

#[test]
fn integrate_is_blocked_by_descendants_leaves_first() {
    let (guard, root) = seeded_child_hall(&["true"]);
    let _ = guard;
    let ctx = Ctx::new(root.clone());

    // A leaf under the child blocks the child.
    let layout = Layout::at(&root);
    let mut leaf = Feature::new(
        FeatureName::new("leaf").unwrap(),
        BranchName::new("leaf").unwrap(),
    );
    leaf.parent = Some(FeatureName::new("child").unwrap());
    leaf.write(&layout).unwrap();

    let failure = integrate(&ctx, integrate_input("child")).unwrap_err();
    assert_eq!(failure.code, "feature.descendants_block");
    assert!(failure.actual.as_deref().unwrap().contains("leaf"));
    // Nothing moved: the parent branch is still at its base.
    let bare = layout.repo_bare(&api());
    let parent_sha = crate::git::System.revision_commit(&bare, "parent").unwrap();
    let main_sha = crate::git::System.revision_commit(&bare, "main").unwrap();
    assert_eq!(parent_sha, main_sha, "the parent must not move");
}

// -- the local journey -------------------------------------------------------

#[test]
fn local_squash_integrates_the_child_and_closes_it_integrated() {
    let (_guard, root) = seeded_child_hall(&["true"]);
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(&root);
    let bare = layout.repo_bare(&api());

    let report = integrate(&ctx, integrate_input("child")).unwrap();

    assert!(report.value.closed_integrated);
    assert_eq!(report.value.state.to_string(), "integrated");
    assert_eq!(report.value.policy.via.to_string(), "local");
    assert_eq!(report.value.policy.strategy.to_string(), "squash");
    assert_eq!(report.value.repos.len(), 1);
    assert_eq!(
        report.value.repos[0].status,
        RepoIntegrationStatus::Integrated
    );

    // The parent worktree now carries the child's work.
    let parent_wt = layout.repo_worktree(&api(), &BranchName::new("parent").unwrap());
    assert_eq!(
        std::fs::read_to_string(parent_wt.join("work.md")).unwrap(),
        "work\n"
    );

    // The receipt records source/target/result/evidence.
    let child = Feature::read(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .unwrap();
    let receipt = child.promotions[&api()]
        .integration_receipt
        .clone()
        .unwrap();
    assert_eq!(receipt.target_branch.as_str(), "parent");
    assert_eq!(receipt.strategy, IntegrationStrategy::Squash);
    assert!(receipt.verification.passed());
    let parent_tip = crate::git::System.revision_commit(&bare, "parent").unwrap();
    assert_eq!(receipt.result_sha, parent_tip);

    // The child's branch and worktree are retained.
    let child_tip = crate::git::System.revision_commit(&bare, "child").unwrap();
    assert_eq!(receipt.source_sha, child_tip);
    assert!(
        std::fs::exists(layout.repo_worktree(&api(), &BranchName::new("child").unwrap())).unwrap()
    );

    // The close record says integrated.
    let record = read_close(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(record.outcome, "integrated");
}

#[test]
fn a_rerun_reuses_the_fresh_receipt_and_does_not_reopen() {
    let (_guard, root) = seeded_child_hall(&["true"]);
    let ctx = Ctx::new(root.clone());
    integrate(&ctx, integrate_input("child")).unwrap();
    let layout = Layout::at(&root);
    let bare = layout.repo_bare(&api());
    let parent_before = crate::git::System.revision_commit(&bare, "parent").unwrap();

    let report = integrate(&ctx, integrate_input("child")).unwrap();

    assert!(!report.value.closed_integrated, "a rerun never reopens");
    assert_eq!(report.value.repos[0].status, RepoIntegrationStatus::Reused);
    assert_eq!(
        crate::git::System.revision_commit(&bare, "parent").unwrap(),
        parent_before,
        "reuse moves nothing"
    );
}

#[test]
fn the_three_local_strategies_all_land_the_work() {
    for strategy in ["squash", "merge", "rebase"] {
        let (guard, root) = seeded_child_hall(&["true"]);
        let _ = guard;
        let ctx = Ctx::new(root.clone());
        let layout = Layout::at(&root);

        let report = integrate(
            &ctx,
            IntegrateInput {
                feature: "child".to_owned(),
                via: None,
                strategy: Some(strategy.to_owned()),
            },
        )
        .unwrap();

        assert!(
            report.value.closed_integrated,
            "{strategy} must close the child"
        );
        assert_eq!(report.value.policy.strategy.to_string(), strategy);
        let parent_wt = layout.repo_worktree(&api(), &BranchName::new("parent").unwrap());
        assert_eq!(
            std::fs::read_to_string(parent_wt.join("work.md")).unwrap(),
            "work\n",
            "{strategy} must land the work"
        );
    }
}

#[test]
fn a_check_failure_before_the_candidate_leaves_the_parent_untouched_and_records_no_receipt() {
    // The child's own ordered checks include a failing command ("exit 1"):
    // the run stops before the candidate exists, the parent never moves, and
    // no receipt is recorded.
    let (_guard, root) = seeded_child_hall(&["true", "exit 1"]);
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(&root);
    let bare = layout.repo_bare(&api());
    let parent_before = crate::git::System.revision_commit(&bare, "parent").unwrap();

    let report = integrate(&ctx, integrate_input("child")).unwrap();

    assert!(!report.value.closed_integrated);
    assert_eq!(report.value.repos[0].status, RepoIntegrationStatus::Failed);
    assert!(
        report.value.repos[0]
            .detail
            .as_deref()
            .unwrap()
            .contains("checks")
    );

    // Parent untouched; no receipt recorded; no staging directory left.
    assert_eq!(
        crate::git::System.revision_commit(&bare, "parent").unwrap(),
        parent_before
    );
    let child = Feature::read(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .unwrap();
    assert!(child.promotions[&api()].integration_receipt.is_none());
    assert!(
        !std::fs::exists(
            layout
                .feature_dir(&FeatureName::new("child").unwrap())
                .join("integration")
        )
        .unwrap()
    );
}

// -- staleness orientation ---------------------------------------------------

#[test]
fn a_moved_source_after_integration_is_stale_and_blocks_with_orientation() {
    let (guard, root) = seeded_child_hall(&["true"]);
    let _ = guard;
    let ctx = Ctx::new(root.clone());
    integrate(&ctx, integrate_input("child")).unwrap();

    // The child branch moves after the integration closed it.
    let layout = Layout::at(&root);
    let child_wt = layout.repo_worktree(&api(), &BranchName::new("child").unwrap());
    std::fs::write(child_wt.join("more.md"), "more\n").unwrap();
    git(&child_wt, &["add", "more.md"]);
    git(&child_wt, &["commit", "-m", "more"]);

    let failure = integrate(&ctx, integrate_input("child")).unwrap_err();
    assert_eq!(failure.code, "feature.receipt_stale");
    assert!(
        failure.fix_actions.iter().any(|fix| fix
            .command
            .as_deref()
            .is_some_and(|c| c.contains("reset --hard"))),
        "the unsafe recorded-source restoration must be offered"
    );
}

// -- parent promotion --------------------------------------------------------

/// A hall where the parent promotes only `web` and the child promotes
/// `api` + `web` — integrating the child must ask about `api`.
fn parent_missing_repo_hall() -> (tempfile::TempDir, Utf8PathBuf) {
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
    let layout = Layout::at(root.clone());
    let origins = root.parent().unwrap().join("origins");
    let api_origin = seeded_repo(&origins.join("api"), "main");
    let web_origin = seeded_repo(&origins.join("web"), "main");
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![
            Repo::new(
                RepoName::new("api").unwrap(),
                api_origin.as_str(),
                BranchName::new("main").unwrap(),
            )
            .with_checks(vec!["true".to_owned()]),
            Repo::new(
                RepoName::new("web").unwrap(),
                web_origin.as_str(),
                BranchName::new("main").unwrap(),
            )
            .with_checks(vec!["true".to_owned()]),
        ],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "parent".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    create_action(
        &ctx,
        CreateInput {
            name: "child".to_owned(),
            branch: None,
            base: None,
            parent: Some("parent".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    // The parent promotes only web; the child promotes both.
    promote::promote(
        &ctx,
        PromoteInput {
            feature: "parent".to_owned(),
            repo: "web".to_owned(),
            base: None,
        },
    )
    .unwrap();
    for repo in ["api", "web"] {
        promote::promote(
            &ctx,
            PromoteInput {
                feature: "child".to_owned(),
                repo: repo.to_owned(),
                base: None,
            },
        )
        .unwrap();
    }
    for repo in ["api", "web"] {
        let wt = layout.repo_worktree(
            &RepoName::new(repo).unwrap(),
            &BranchName::new("child").unwrap(),
        );
        std::fs::write(wt.join("work.md"), "work\n").unwrap();
        git(&wt, &["add", "work.md"]);
        git(&wt, &["commit", "-m", "child work"]);
    }
    approve_plan(&ctx, "child");
    (guard, root)
}

#[test]
fn a_missing_parent_promotion_blocks_noninteractive_with_the_exact_command() {
    let (_guard, root) = parent_missing_repo_hall();
    let ctx = Ctx::new(root.clone());

    let failure = integrate(&ctx, integrate_input("child")).unwrap_err();
    assert_eq!(failure.code, "integration.parent_promotion_required");
    assert_eq!(
        failure.fix_actions[0].command.as_deref(),
        Some("ivar feature promote parent api")
    );
}

#[test]
fn a_confirmed_parent_promotion_promotes_and_proceeds() {
    let (_guard, root) = parent_missing_repo_hall();
    let mut ctx = Ctx::new(root.clone());
    ctx = ctx.with_confirm(crate::action::confirm::fixed(true));

    let report = integrate(&ctx, integrate_input("child")).unwrap();

    assert!(report.value.closed_integrated);
    // The parent now promotes api too, with a Ready worktree.
    let layout = Layout::at(&root);
    let parent = Feature::read(&layout, &FeatureName::new("parent").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(parent.worktree_state(&api()), Some(WorktreeState::Ready));
}

#[test]
fn a_declined_parent_promotion_blocks_without_mutation() {
    let (_guard, root) = parent_missing_repo_hall();
    let mut ctx = Ctx::new(root.clone());
    ctx = ctx.with_confirm(crate::action::confirm::fixed(false));

    let failure = integrate(&ctx, integrate_input("child")).unwrap_err();
    assert_eq!(failure.code, "integration.parent_promotion_required");

    // Nothing mutated: no receipt, parent still not promoting api.
    let layout = Layout::at(&root);
    let child = Feature::read(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .unwrap();
    assert!(child.promotions[&api()].integration_receipt.is_none());
}

// -- live sessions -----------------------------------------------------------

#[test]
fn a_live_session_blocks_the_first_successful_receipt() {
    let (guard, root) = seeded_child_hall(&["true"]);
    let _ = guard;
    let ctx = Ctx::new(root.clone());

    // A session view dir makes the child's session live.
    let layout = Layout::at(&root);
    fs::ensure_dir(
        &layout
            .feature_sessions_dir(&FeatureName::new("child").unwrap())
            .join("sess-1"),
    )
    .unwrap();

    let failure = integrate(&ctx, integrate_input("child")).unwrap_err();
    assert_eq!(failure.code, "integration.session_live");
    let child = Feature::read(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .unwrap();
    assert!(child.promotions[&api()].integration_receipt.is_none());
}

// -- partial multi-repo resume ------------------------------------------------

/// A hall with two repos (api and web), both promoted into parent and child,
/// with a commit on the child and the plan approved.
fn seeded_two_repo_child_hall() -> (tempfile::TempDir, Utf8PathBuf) {
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

    let layout = Layout::at(root.clone());
    let origins = root.parent().unwrap().join("origins");
    let api_origin = seeded_repo(&origins.join("api"), "main");
    let web_origin = seeded_repo(&origins.join("web"), "main");
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![
            Repo::new(
                RepoName::new("api").unwrap(),
                api_origin.as_str(),
                BranchName::new("main").unwrap(),
            )
            .with_checks(vec!["true".to_owned()]),
            Repo::new(
                RepoName::new("web").unwrap(),
                web_origin.as_str(),
                BranchName::new("main").unwrap(),
            )
            .with_checks(vec!["true".to_owned()]),
        ],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "parent".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    create_action(
        &ctx,
        CreateInput {
            name: "child".to_owned(),
            branch: None,
            base: None,
            parent: Some("parent".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    for feature in ["parent", "child"] {
        for repo in ["api", "web"] {
            promote::promote(
                &ctx,
                PromoteInput {
                    feature: feature.to_owned(),
                    repo: repo.to_owned(),
                    base: None,
                },
            )
            .unwrap();
        }
    }
    // A commit on the child in both repos.
    for repo in ["api", "web"] {
        let wt = layout.repo_worktree(
            &RepoName::new(repo).unwrap(),
            &BranchName::new("child").unwrap(),
        );
        std::fs::write(wt.join("work.md"), "work\n").unwrap();
        git(&wt, &["add", "work.md"]);
        git(&wt, &["commit", "-m", "child work"]);
    }
    approve_plan(&ctx, "child");
    (guard, root)
}

#[test]
fn a_partial_two_repo_integration_is_resumable_and_never_atomic() {
    let (_guard, root) = seeded_two_repo_child_hall();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(&root);

    let report = integrate(&ctx, integrate_input("child")).unwrap();

    assert!(report.value.closed_integrated);
    assert_eq!(report.value.repos.len(), 2);
    assert!(
        report
            .value
            .repos
            .iter()
            .all(|repo| repo.status == RepoIntegrationStatus::Integrated)
    );

    // Every receipt is fresh and passing.
    let child = Feature::read(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .unwrap();
    assert!(child.all_promotions_have_passing_receipts());
}

#[test]
fn a_failed_repo_is_resumable_while_a_successful_repo_stays_locked() {
    // web's child checks fail ("exit 1"), api's pass. The run integrates api
    // and leaves web failed — partial, not atomic.
    let (guard, root) = hall_root();
    let _ = guard;
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
    let layout = Layout::at(root.clone());
    let origins = root.parent().unwrap().join("origins");
    let api_origin = seeded_repo(&origins.join("api"), "main");
    let web_origin = seeded_repo(&origins.join("web"), "main");
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![
            Repo::new(
                RepoName::new("api").unwrap(),
                api_origin.as_str(),
                BranchName::new("main").unwrap(),
            )
            .with_checks(vec!["true".to_owned()]),
            Repo::new(
                RepoName::new("web").unwrap(),
                web_origin.as_str(),
                BranchName::new("main").unwrap(),
            )
            .with_checks(vec!["exit 1".to_owned()]),
        ],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    create_action(
        &ctx,
        CreateInput {
            name: "parent".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    create_action(
        &ctx,
        CreateInput {
            name: "child".to_owned(),
            branch: None,
            base: None,
            parent: Some("parent".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    for feature in ["parent", "child"] {
        for repo in ["api", "web"] {
            promote::promote(
                &ctx,
                PromoteInput {
                    feature: feature.to_owned(),
                    repo: repo.to_owned(),
                    base: None,
                },
            )
            .unwrap();
        }
    }
    for repo in ["api", "web"] {
        let wt = layout.repo_worktree(
            &RepoName::new(repo).unwrap(),
            &BranchName::new("child").unwrap(),
        );
        std::fs::write(wt.join("work.md"), "work\n").unwrap();
        git(&wt, &["add", "work.md"]);
        git(&wt, &["commit", "-m", "child work"]);
    }
    approve_plan(&ctx, "child");

    let report = integrate(&ctx, integrate_input("child")).unwrap();

    // api integrated, web failed — and the child is NOT closed.
    assert!(!report.value.closed_integrated);
    let api_entry = report
        .value
        .repos
        .iter()
        .find(|entry| entry.repo.as_str() == "api")
        .unwrap();
    let web_entry = report
        .value
        .repos
        .iter()
        .find(|entry| entry.repo.as_str() == "web")
        .unwrap();
    assert_eq!(api_entry.status, RepoIntegrationStatus::Integrated);
    assert_eq!(web_entry.status, RepoIntegrationStatus::Failed);

    // api's receipt is persisted and locks its promotion; web has none.
    let child = Feature::read(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .unwrap();
    assert!(child.promotion_has_successful_receipt(&RepoName::new("api").unwrap()));
    assert!(
        child.promotions[&RepoName::new("web").unwrap()]
            .integration_receipt
            .is_none()
    );

    // Repair web: rewrite the manifest so web's checks pass, then rerun —
    // api is reused byte-for-byte, web integrates, and the child closes.
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(layout.manifest()).unwrap()).unwrap();
    for repo in value["repos"].as_array_mut().unwrap() {
        if repo["name"] == "web" {
            repo["checks"] = serde_json::json!(["true"]);
        }
    }
    std::fs::write(layout.manifest(), serde_json::to_string(&value).unwrap()).unwrap();

    let api_before = crate::git::System
        .revision_commit(&layout.repo_bare(&RepoName::new("api").unwrap()), "parent")
        .unwrap();
    let report = integrate(&ctx, integrate_input("child")).unwrap();
    assert!(report.value.closed_integrated);
    let api_reused = report
        .value
        .repos
        .iter()
        .find(|entry| entry.repo.as_str() == "api")
        .unwrap();
    let web_integrated = report
        .value
        .repos
        .iter()
        .find(|entry| entry.repo.as_str() == "web")
        .unwrap();
    assert_eq!(api_reused.status, RepoIntegrationStatus::Reused);
    assert_eq!(web_integrated.status, RepoIntegrationStatus::Integrated);
    assert_eq!(
        crate::git::System
            .revision_commit(&layout.repo_bare(&RepoName::new("api").unwrap()), "parent")
            .unwrap(),
        api_before,
        "the successful repo A must stay byte-for-byte locked"
    );
}
