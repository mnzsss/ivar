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
use crate::domain::feature::{DeliveryAction, DeliveryRepo};
use crate::domain::name::{BranchName, HallName};
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{git, hall_root, seeded_repo};
use camino::Utf8Path;

/// A hall with `repos` declared, a `checkout` feature, every repo promoted,
/// and one commit on each feature branch so there is something to deliver.
/// Walk `checkout` through the SPDD gates up to and including `plan`, using
/// the real plan actions. Apply refuses without this.
fn approve_through_plan(root: &Utf8PathBuf) {
    let ctx = Ctx::new(root.clone());
    crate::action::plan::create::create(
        &ctx,
        crate::action::plan::create::CreateInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    for gate in ["requirements", "analysis", "plan"] {
        crate::action::plan::approve::approve(
            &ctx,
            crate::action::plan::approve::ApproveInput {
                feature: "checkout".to_owned(),
                gate: gate.to_owned(),
            },
        )
        .unwrap();
    }
}

fn hall_with_promoted(repos: &[&str]) -> (tempfile::TempDir, Utf8PathBuf) {
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

    let origins = root.parent().unwrap().join("origins");
    let declared: Vec<Repo> = repos
        .iter()
        .map(|name| {
            let origin = seeded_repo(&origins.join(name), "main");
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
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    let branch = BranchName::new("checkout").unwrap();
    for name in repos {
        promote::promote(
            &ctx,
            PromoteInput {
                feature: "checkout".to_owned(),
                repo: (*name).to_owned(),
            },
        )
        .unwrap();
        let worktree = layout.repo_worktree(&RepoName::new(*name).unwrap(), &branch);
        std::fs::write(worktree.join("work.md"), "work\n").unwrap();
        git(&worktree, &["add", "work.md"]);
        git(&worktree, &["commit", "-m", "work"]);
    }

    (guard, root)
}

fn preview_input(feature: &str) -> DeliverInput {
    DeliverInput {
        feature: feature.to_owned(),
        preview: true,
        fingerprint: None,
    }
}

fn apply_input(feature: &str, fingerprint: &str) -> DeliverInput {
    DeliverInput {
        feature: feature.to_owned(),
        preview: false,
        fingerprint: Some(fingerprint.to_owned()),
    }
}

/// The remote's view of one branch: the `ls-remote` line, or `None` when
/// the branch is not there.
fn remote_ref(origin: &str, branch: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["ls-remote", origin, &format!("refs/heads/{branch}")])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "ls-remote failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        None
    } else {
        Some(stdout.trim().to_owned())
    }
}

fn origin_of(root: &Utf8Path, repo: &str) -> String {
    let layout = Layout::at(root.to_path_buf());
    Manifest::read(&layout)
        .unwrap()
        .unwrap()
        .repos()
        .iter()
        .find(|declared| declared.name().as_str() == repo)
        .unwrap()
        .url()
        .to_owned()
}

// -- preview --------------------------------------------------------------

#[test]
fn preview_lists_every_promoted_repo_with_its_delivery_facts() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());

    let report = deliver(&ctx, preview_input("checkout")).unwrap();

    assert!(report.is_clean());
    assert!(report.value.pushes.is_empty(), "preview must not push");
    assert_eq!(report.value.preview.repos.len(), 1);
    let repo = &report.value.preview.repos[0];
    assert_eq!(repo.repo.as_str(), "api");
    assert_eq!(repo.local_branch.as_str(), "checkout");
    assert!(repo.remote.contains("origins/api"), "was: {}", repo.remote);
    assert_eq!(repo.push_refspec, "checkout:refs/heads/checkout");
    assert_eq!(repo.action, DeliveryAction::PushOnly);
    assert_eq!(repo.base_branch.as_str(), "main");
    assert!(repo.dependencies.is_empty());
    // One commit beyond main, no upstream: the unpushed blocker.
    assert!(
        repo.blockers
            .iter()
            .any(|blocker| blocker.contains("1 commit(s) not pushed")),
        "was: {:?}",
        repo.blockers
    );
    // Preview is side-effect-free: the remote has no branch yet.
    assert!(remote_ref(&origin_of(&root, "api"), "checkout").is_none());
}

#[test]
fn the_preview_has_a_stable_content_fingerprint() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());

    let first = deliver(&ctx, preview_input("checkout")).unwrap();
    let second = deliver(&ctx, preview_input("checkout")).unwrap();

    let fingerprint = &first.value.preview.fingerprint;
    assert_eq!(fingerprint.len(), 64, "a sha-256 hex digest");
    assert_eq!(fingerprint, &second.value.preview.fingerprint);
}

#[test]
fn a_feature_with_no_promoted_repos_previews_empty() {
    let (_guard, root) = hall_with_promoted(&[]);
    let ctx = Ctx::new(root.clone());

    let report = deliver(&ctx, preview_input("checkout")).unwrap();

    assert!(report.value.preview.repos.is_empty());
    assert_eq!(report.value.preview.fingerprint.len(), 64);
}

#[test]
fn a_dirty_worktree_is_listed_as_a_blocker() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    let worktree = Layout::at(root.clone()).repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(worktree.join("notes.md"), "mine\n").unwrap();

    let report = deliver(&ctx, preview_input("checkout")).unwrap();

    let repo = &report.value.preview.repos[0];
    assert!(
        repo.blockers
            .iter()
            .any(|blocker| blocker.contains("uncommitted changes")),
        "was: {:?}",
        repo.blockers
    );
}

#[test]
fn delivering_a_missing_feature_is_blocked() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root);

    let failure = deliver(&ctx, preview_input("ghost")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.not_found");
}

// -- apply: gating --------------------------------------------------------

#[test]
fn apply_requires_a_preview_fingerprint() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            fingerprint: None,
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "deliver.preview_required");
}

#[test]
fn apply_is_rejected_when_the_state_has_drifted_since_the_preview() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let approved = deliver(&ctx, preview_input("checkout")).unwrap();
    let fingerprint = approved.value.preview.fingerprint.clone();

    // Drift: one more commit lands on the feature branch.
    let worktree = Layout::at(root.clone()).repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(worktree.join("more.md"), "more\n").unwrap();
    git(&worktree, &["add", "more.md"]);
    git(&worktree, &["commit", "-m", "more"]);

    let failure = deliver(&ctx, apply_input("checkout", &fingerprint)).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "deliver.fingerprint_mismatch");
    assert!(
        failure
            .fix_actions
            .iter()
            .any(|fix| fix.code == "deliver.re_preview"),
        "the fix must re-run the preview: {:?}",
        failure.fix_actions
    );
    // Nothing was pushed.
    assert!(remote_ref(&origin_of(&root, "api"), "checkout").is_none());
}

// -- apply: pushing -------------------------------------------------------

#[test]
fn deliver_pushes_the_feature_branch_to_the_remote() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let approved = deliver(&ctx, preview_input("checkout")).unwrap();
    let fingerprint = approved.value.preview.fingerprint.clone();

    let report = deliver(&ctx, apply_input("checkout", &fingerprint)).unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.pushes.len(), 1);
    assert!(report.value.pushes[0].ok);
    assert_eq!(report.value.pushes[0].repo.as_str(), "api");
    // The remote now holds the branch, at the tip that was previewed.
    assert!(remote_ref(&origin_of(&root, "api"), "checkout").is_some());
}

#[test]
fn a_failed_push_is_a_warning_and_does_not_block_the_others() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    approve_through_plan(&root);
    // Break web's remote before previewing, so the approved state says the
    // bogus URL — the fingerprint then matches when apply runs.
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let repos: Vec<Repo> = manifest
        .repos()
        .iter()
        .map(|repo| {
            if repo.name().as_str() == "web" {
                Repo::new(
                    RepoName::new("web").unwrap(),
                    root.join("no-such-origin").as_str(),
                    BranchName::new("main").unwrap(),
                )
            } else {
                repo.clone()
            }
        })
        .collect();
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        repos,
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    let ctx = Ctx::new(root.clone());

    let approved = deliver(&ctx, preview_input("checkout")).unwrap();
    let report = deliver(
        &ctx,
        apply_input("checkout", &approved.value.preview.fingerprint),
    )
    .unwrap();

    assert!(!report.is_clean(), "a failed push must not be a clean run");
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].subject, "web");
    assert_eq!(report.warnings[0].code, "deliver.push_failed");
    // Best-effort: api still landed.
    assert!(
        report
            .value
            .pushes
            .iter()
            .any(|push| push.repo.as_str() == "api" && push.ok)
    );
    assert!(
        report
            .value
            .pushes
            .iter()
            .any(|push| push.repo.as_str() == "web" && !push.ok)
    );
    assert!(remote_ref(&origin_of(&root, "api"), "checkout").is_some());
}

// -- ordering -------------------------------------------------------------

fn delivery_repo(name: &str, dependencies: Vec<&str>) -> DeliveryRepo {
    DeliveryRepo {
        repo: RepoName::new(name).unwrap(),
        local_branch: BranchName::new("checkout").unwrap(),
        remote: "git@example.com:acme/api.git".to_owned(),
        push_refspec: "checkout:refs/heads/checkout".to_owned(),
        action: DeliveryAction::PushOnly,
        base_branch: BranchName::new("main").unwrap(),
        dependencies: dependencies
            .into_iter()
            .map(|dep| RepoName::new(dep).unwrap())
            .collect(),
        blockers: Vec::new(),
        pr_url: None,
    }
}

#[test]
fn ordering_puts_a_repos_dependencies_before_it() {
    let mut repos = vec![
        delivery_repo("api", vec!["web"]),
        delivery_repo("web", vec![]),
        delivery_repo("cron", vec![]),
    ];

    order_by_dependencies(&mut repos);

    let order: Vec<&str> = repos.iter().map(|repo| repo.repo.as_str()).collect();
    let web = order.iter().position(|name| *name == "web").unwrap();
    let api = order.iter().position(|name| *name == "api").unwrap();
    assert!(web < api, "a dependency must be pushed first: {order:?}");
    assert_eq!(order.len(), 3);
}

#[test]
fn ordering_preserves_name_order_between_unrelated_repos() {
    let mut repos = vec![
        delivery_repo("b", vec![]),
        delivery_repo("a", vec![]),
        delivery_repo("c", vec![]),
    ];

    order_by_dependencies(&mut repos);

    let order: Vec<&str> = repos.iter().map(|repo| repo.repo.as_str()).collect();
    assert_eq!(order, vec!["b", "a", "c"], "no dependencies, no reordering");
}

// -- rendering ------------------------------------------------------------

#[test]
fn the_human_preview_surface_lists_each_repo_and_the_fingerprint() {
    let outcome = DeliverOutcome {
        root: Utf8PathBuf::from("/hall"),
        preview: DeliveryPreview {
            feature: FeatureName::new("checkout").unwrap(),
            plan_gate: GateState::Approved,
            repos: vec![delivery_repo("api", vec![])],
            fingerprint: "abc123".to_owned(),
        },
        pushes: Vec::new(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("Delivery preview for `checkout` in /hall:"));
    assert!(rendered.contains("branch:  checkout"));
    assert!(rendered.contains("refspec: checkout:refs/heads/checkout"));
    assert!(rendered.contains("base:    main"));
    assert!(rendered.contains("action:  push only"));
    assert!(rendered.contains("blockers: none"));
    assert!(rendered.contains("fingerprint: abc123"));
}

#[test]
fn the_human_apply_surface_reports_each_push() {
    let outcome = DeliverOutcome {
        root: Utf8PathBuf::from("/hall"),
        preview: DeliveryPreview {
            feature: FeatureName::new("checkout").unwrap(),
            plan_gate: GateState::Approved,
            repos: vec![delivery_repo("api", vec![])],
            fingerprint: "abc123".to_owned(),
        },
        pushes: vec![
            PushResult {
                repo: RepoName::new("api").unwrap(),
                ok: true,
                detail: None,
            },
            PushResult {
                repo: RepoName::new("web").unwrap(),
                ok: false,
                detail: Some("remote did not answer".to_owned()),
            },
        ],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("Delivered `checkout` in /hall (fingerprint abc123):"));
    assert!(rendered.contains("  api: pushed"));
    assert!(rendered.contains("  web: not pushed — remote did not answer"));
}
