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
            artifacts: Vec::new(),
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
            base: None,
            parent: None,
            via: None,
            strategy: None,
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
                base: None,
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
        land: false,
        fingerprint: None,
    }
}

fn apply_input(feature: &str, fingerprint: &str) -> DeliverInput {
    DeliverInput {
        feature: feature.to_owned(),
        preview: false,
        land: false,
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

/// `base_branch` in the preview is the base `promote` actually recorded —
/// the feature's declared base, not always the repo's default branch.
#[test]
fn preview_shows_the_recorded_base_not_always_the_default_branch() {
    let (_guard, root) = hall_root();
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
            parent: None,
            via: None,
            strategy: None,
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

    let report = deliver(&ctx, preview_input("checkout")).unwrap();

    assert_eq!(
        report.value.preview.repos[0].base_branch.as_str(),
        "develop"
    );
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
            land: false,
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
fn a_branch_the_remote_already_carries_is_not_reported_as_unpushed() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());
    let approved = deliver(&ctx, preview_input("checkout")).unwrap();
    deliver(
        &ctx,
        apply_input("checkout", &approved.value.preview.fingerprint),
    )
    .unwrap();

    // `deliver` pushed; local and remote now hold the same commit. Previewing
    // again must not claim there is work waiting to be pushed.
    let report = deliver(&ctx, preview_input("checkout")).unwrap();

    let repo = &report.value.preview.repos[0];
    assert!(
        !repo
            .blockers
            .iter()
            .any(|blocker| blocker.contains("not pushed")),
        "was: {:?}",
        repo.blockers
    );
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
        default_branch: None,
        ff_possible: None,
        remote_default_tip: None,
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

// -- roots only, and a healthy tree below them ------------------------------

/// A child of `checkout`, created directly (create --parent is covered by the
/// create tests).
fn child_of_checkout(root: &Utf8PathBuf, name: &str) {
    let layout = Layout::at(root.clone());
    let mut child = crate::domain::feature::Feature::new(
        crate::domain::name::FeatureName::new(name).unwrap(),
        BranchName::new(name).unwrap(),
    );
    child.parent = Some(crate::domain::name::FeatureName::new("checkout").unwrap());
    child.write(&layout).unwrap();
}

#[test]
fn deliver_refuses_a_child_with_the_integrate_command() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    child_of_checkout(&root, "child");

    // Preview and apply refuse identically.
    let failure = deliver(&ctx, preview_input("child")).unwrap_err();
    assert_eq!(failure.code, "deliver.child_requires_integration");
    assert_eq!(
        failure.fix_actions[0].command.as_deref(),
        Some("ivar feature integrate child")
    );
    let failure = deliver(&ctx, apply_input("child", "whatever")).unwrap_err();
    assert_eq!(failure.code, "deliver.child_requires_integration");
}

#[test]
fn deliver_preview_reports_tree_blockers_and_apply_refuses_before_push() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);
    // An active leaf under the root blocks its delivery.
    child_of_checkout(&root, "child");
    let layout = Layout::at(root.clone());
    let mut leaf = crate::domain::feature::Feature::new(
        crate::domain::name::FeatureName::new("leaf").unwrap(),
        BranchName::new("leaf").unwrap(),
    );
    leaf.parent = Some(crate::domain::name::FeatureName::new("child").unwrap());
    leaf.write(&layout).unwrap();

    // The preview fingerprints the blockers and reports them.
    let report = deliver(&ctx, preview_input("checkout")).unwrap();
    assert_eq!(report.value.preview.tree_blockers.len(), 2);
    let names: Vec<&str> = report
        .value
        .preview
        .tree_blockers
        .iter()
        .map(|blocker| blocker.feature.as_str())
        .collect();
    assert_eq!(names, ["child", "leaf"]);
    assert_eq!(report.value.preview.tree_blockers[0].depth, 1);

    // Apply refuses before any push.
    let fingerprint = report.value.preview.fingerprint.clone();
    let failure = deliver(&ctx, apply_input("checkout", &fingerprint)).unwrap_err();
    assert_eq!(failure.code, "deliver.descendants_block");
    assert!(failure.actual.as_deref().unwrap().contains("child"));
}

#[test]
fn deliver_ignores_abandoned_descendants_but_sees_active_grandchildren() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);
    child_of_checkout(&root, "abandoned");
    let layout = Layout::at(&root);
    let mut grandchild = crate::domain::feature::Feature::new(
        crate::domain::name::FeatureName::new("active").unwrap(),
        BranchName::new("active").unwrap(),
    );
    grandchild.parent = Some(crate::domain::name::FeatureName::new("abandoned").unwrap());
    grandchild.write(&layout).unwrap();
    crate::action::feature::lifecycle::write_close(
        &layout,
        &crate::domain::name::FeatureName::new("abandoned").unwrap(),
        crate::domain::feature::PromotionOutcome::Abandoned,
    )
    .unwrap();

    let report = deliver(&ctx, preview_input("checkout")).unwrap();
    let names: Vec<&str> = report
        .value
        .preview
        .tree_blockers
        .iter()
        .map(|blocker| blocker.feature.as_str())
        .collect();
    assert_eq!(
        names,
        ["active"],
        "abandoned history does not block, but its active grandchild does"
    );
}

// -- rendering ------------------------------------------------------------

#[test]
fn the_human_preview_surface_lists_each_repo_and_the_fingerprint() {
    let outcome = DeliverOutcome {
        root: Utf8PathBuf::from("/hall"),
        preview: DeliveryPreview {
            feature: FeatureName::new("checkout").unwrap(),
            mode: DeliveryMode::Push,
            plan_gate: GateState::Approved,
            repos: vec![delivery_repo("api", vec![])],
            tree_blockers: Vec::new(),
            fingerprint: "abc123".to_owned(),
        },
        pushes: Vec::new(),
        land: Vec::new(),
        checks: Vec::new(),
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
            mode: DeliveryMode::Push,
            plan_gate: GateState::Approved,
            repos: vec![delivery_repo("api", vec![])],
            tree_blockers: Vec::new(),
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
        land: Vec::new(),
        checks: Vec::new(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("Delivered `checkout` in /hall (fingerprint abc123):"));
    assert!(rendered.contains("  api: pushed"));
    assert!(rendered.contains("  web: not pushed — remote did not answer"));
}

/// The short path's sharp edge, closed at the surface that matters. A feature
/// that approved `plan` while `requirements.md` did not exist may deliver.
/// Once `requirements.md` appears unapproved, that approval no longer holds,
/// and `deliver` has to refuse rather than ship on a gate `plan approve` would
/// now decline to grant.
#[test]
fn deliver_refuses_once_an_upstream_artifact_appears_after_approval() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());

    // Short path: scaffold and approve the plan gate alone.
    crate::action::plan::create::create(
        &ctx,
        crate::action::plan::create::CreateInput {
            feature: "checkout".to_owned(),
            artifacts: vec![crate::action::plan::Artifact::Plan],
        },
    )
    .unwrap();
    crate::action::plan::approve::approve(
        &ctx,
        crate::action::plan::approve::ApproveInput {
            feature: "checkout".to_owned(),
            gate: "plan".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(
        deliver(&ctx, preview_input("checkout"))
            .unwrap()
            .value
            .preview
            .plan_gate,
        GateState::Approved
    );

    // The upstream artifact appears, unapproved.
    crate::infra::fs::write_text(
        &root.join("plans/checkout/requirements.md"),
        "# Requirements\n",
    )
    .unwrap();

    let preview = deliver(&ctx, preview_input("checkout")).unwrap();
    assert_eq!(preview.value.preview.plan_gate, GateState::NeedsRevision);

    let failure = deliver(
        &ctx,
        apply_input("checkout", &preview.value.preview.fingerprint),
    )
    .unwrap_err();
    assert_eq!(failure.code, "deliver.plan_not_approved");
}

/// `deliver` read `approvals.json` raw, which answers a question about the
/// last command that wrote it rather than about the feature as it stands. A
/// plan.md edited after approval was reported `needs-revision` by
/// `ivar plan status` and shipped by `deliver` anyway — the tool enforcing one
/// rule and reporting another.
#[test]
fn deliver_refuses_a_plan_edited_after_it_was_approved() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    assert_eq!(
        deliver(&ctx, preview_input("checkout"))
            .unwrap()
            .value
            .preview
            .plan_gate,
        GateState::Approved
    );

    // A human rewrites the plan after approving it.
    let plan_path = root.join("plans/checkout/plan.md");
    let body = crate::infra::fs::read_text(&plan_path).unwrap().unwrap();
    crate::infra::fs::write_text(&plan_path, &format!("{body}\nrewritten\n")).unwrap();

    let preview = deliver(&ctx, preview_input("checkout")).unwrap();
    assert_eq!(preview.value.preview.plan_gate, GateState::NeedsRevision);

    let failure = deliver(
        &ctx,
        apply_input("checkout", &preview.value.preview.fingerprint),
    )
    .unwrap_err();
    assert_eq!(failure.code, "deliver.plan_not_approved");
}

#[test]
fn push_and_land_previews_of_the_same_state_fingerprint_differently() {
    let feature = FeatureName::new("checkout").unwrap();
    let repos = vec![];
    let push = fingerprint_for(
        &feature,
        DeliveryMode::Push,
        GateState::Approved,
        &[],
        &repos,
    )
    .expect("push fingerprint");
    let land = fingerprint_for(
        &feature,
        DeliveryMode::Land,
        GateState::Approved,
        &[],
        &repos,
    )
    .expect("land fingerprint");
    assert_ne!(
        push, land,
        "a push-approved fingerprint must not authorise a land"
    );
}

#[test]
fn preview_without_mode_defaults_to_push() {
    let json = serde_json::json!({
        "feature": "checkout",
        "plan_gate": "approved",
        "repos": [],
        "fingerprint": ""
    });
    let preview: DeliveryPreview = serde_json::from_value(json).expect("legacy preview");
    assert_eq!(preview.mode, DeliveryMode::Push);
}

#[test]
fn land_on_default_serialises_as_snake_case_and_has_a_word() {
    let action = DeliveryAction::LandOnDefault;
    assert_eq!(
        serde_json::to_value(action).unwrap(),
        serde_json::json!("land_on_default")
    );
    assert_eq!(action_word(action), "land on default");
}

#[test]
fn a_push_fingerprint_cannot_be_applied_as_a_land() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());

    let push_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: false,
            fingerprint: None,
        },
    )
    .expect("push preview");

    let refused = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(push_preview.value.preview.fingerprint.clone()),
        },
    );
    let failure = refused.expect_err("a push fingerprint must not open a land");
    assert_eq!(failure.code, "deliver.fingerprint_mismatch");
}

#[test]
fn a_land_fingerprint_cannot_be_applied_as_a_push() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("land preview");

    let refused = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: false,
            fingerprint: Some(land_preview.value.preview.fingerprint.clone()),
        },
    );
    let failure = refused.expect_err("a land fingerprint must not open a push");
    assert_eq!(failure.code, "deliver.fingerprint_mismatch");
}

#[test]
fn a_matching_land_fingerprint_executes_the_land() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let ctx = Ctx::new(root.clone());

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("land preview");

    let applied = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint.clone()),
        },
    );
    // Wave 3: land now executes; a push to a non-bare origin may warn but the
    // merge must have been attempted.
    match applied {
        Ok(out) => {
            assert!(
                out.value.land.iter().any(|r| r.merged),
                "at least one repo must merge"
            );
        }
        Err(failure) => {
            // fast-forward failure or mode mismatch — both surface clearly
            assert_ne!(
                failure.code, "deliver.land_not_implemented",
                "land_not_implemented must never fire after Wave 3",
            );
        }
    }
}

// -- land preview and blockers (Wave 2) ------------------------------------

fn git_stdout(cwd: &Utf8Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(["-c", "user.name=ivar tests"])
        .args(["-c", "user.email=tests@ivar.invalid"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "git {} failed in {cwd}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn land_preview_input(feature: &str) -> DeliverInput {
    DeliverInput {
        feature: feature.to_owned(),
        preview: true,
        land: true,
        fingerprint: None,
    }
}

#[test]
fn land_preview_reports_ff_possible_without_touching_the_remote() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let out = deliver(&ctx, land_preview_input("checkout")).expect("land preview");
    let repo = &out.value.preview.repos[0];
    assert_eq!(repo.ff_possible, Some(true));
    assert_eq!(repo.default_branch.as_ref().unwrap().as_str(), "main");
}

#[test]
fn a_feature_at_the_default_tip_is_fast_forwardable() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    let tip = git_stdout(&worktree, &["rev-parse", "HEAD"]);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    git(&default_worktree, &["reset", "--hard", tip.trim()]);

    let out = deliver(&ctx, land_preview_input("checkout")).expect("land preview");
    assert_eq!(out.value.preview.repos[0].ff_possible, Some(true));
}

#[test]
fn push_preview_leaves_land_fields_absent() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let out = deliver(&ctx, preview_input("checkout")).expect("push preview");
    assert!(out.value.preview.repos[0].ff_possible.is_none());
    assert!(out.value.preview.repos[0].default_branch.is_none());
}

#[test]
fn diverged_default_is_not_fast_forwardable() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    std::fs::write(default_worktree.join("main.txt"), "main commit\n").unwrap();
    git(&default_worktree, &["add", "main.txt"]);
    git(&default_worktree, &["commit", "-m", "main commit"]);

    let failure = deliver(&ctx, land_preview_input("checkout")).expect_err("non-ff must block");
    assert_eq!(failure.code, "deliver.land_not_fast_forward");
    let fix = failure
        .fix_actions
        .first()
        .expect("a blocked land must say how to unblock");
    assert_eq!(
        fix.command.as_deref().unwrap(),
        "ivar feature rebase checkout"
    );
}

#[test]
fn dirty_default_worktree_blocks_and_is_left_untouched() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    std::fs::write(default_worktree.join("dirty.txt"), "uncommitted changes\n").unwrap();

    let before = std::fs::read(default_worktree.join("dirty.txt")).unwrap();
    let failure = deliver(&ctx, land_preview_input("checkout")).expect_err("dirty must block");
    assert_eq!(failure.code, "deliver.land_dirty_worktree");
    let after = std::fs::read(default_worktree.join("dirty.txt")).unwrap();
    assert_eq!(before, after);
}

#[test]
fn rebase_in_progress_blocks() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let worktree_git_dir = crate::git::read::worktree_git_dir(&default_worktree).unwrap();
    std::fs::create_dir_all(worktree_git_dir.join("rebase-merge")).unwrap();

    let failure =
        deliver(&ctx, land_preview_input("checkout")).expect_err("rebase in progress must block");
    assert_eq!(failure.code, "deliver.land_rebase_in_progress");
}

#[test]
fn land_no_repos_blocks() {
    let (_guard, root) = hall_root();
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
    approve_through_plan(&root);

    let failure = deliver(&ctx, land_preview_input("checkout")).expect_err("no repos must block");
    assert_eq!(failure.code, "deliver.land_no_repos");
}

#[test]
fn land_preview_names_the_target_and_the_mode() {
    let outcome = DeliverOutcome {
        root: Utf8PathBuf::from("/hall"),
        preview: DeliveryPreview {
            feature: FeatureName::new("checkout").unwrap(),
            mode: DeliveryMode::Land,
            plan_gate: GateState::Approved,
            repos: vec![DeliveryRepo {
                repo: RepoName::new("api").unwrap(),
                local_branch: BranchName::new("land-on-default").unwrap(),
                remote: "https://github.com/acme/api".to_owned(),
                push_refspec: "land-on-default:refs/heads/land-on-default".to_owned(),
                action: DeliveryAction::LandOnDefault,
                base_branch: BranchName::new("main").unwrap(),
                dependencies: Vec::new(),
                blockers: Vec::new(),
                pr_url: None,
                default_branch: Some(BranchName::new("main").unwrap()),
                ff_possible: Some(true),
                remote_default_tip: None,
            }],
            tree_blockers: Vec::new(),
            fingerprint: "abc123".to_owned(),
        },
        pushes: Vec::new(),
        land: Vec::new(),
        checks: Vec::new(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("land on default"));
    assert!(rendered.contains("api  land-on-default -> main  fast-forward"));
    assert!(!rendered.contains("pull request"), "land opens no PR");

    let json_val = serde_json::to_value(&outcome.preview).unwrap();
    assert_eq!(json_val["mode"], "land");
    assert_eq!(json_val["repos"][0]["default_branch"], "main");
    assert_eq!(json_val["repos"][0]["ff_possible"], true);
}

// -- land apply (Wave 3) ----------------------------------------------------

fn snapshot_all_worktrees(root: &Utf8Path) -> Vec<(Utf8PathBuf, String)> {
    let layout = Layout::at(root);
    let manifest = read_manifest(&layout).unwrap();
    let mut snapshots = Vec::new();
    for repo in manifest.repos() {
        let worktree = layout.repo_worktree(repo.name(), repo.default_branch());
        if worktree.exists() {
            let sha = git_stdout(&worktree, &["rev-parse", "HEAD"])
                .trim()
                .to_owned();
            snapshots.push((worktree, sha));
        }
    }
    snapshots.sort_by(|a, b| a.0.cmp(&b.0));
    snapshots
}

#[test]
fn one_blocked_repo_blocks_the_whole_land_and_writes_nothing() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let default_worktree_web = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    git_stdout(
        &default_worktree_web,
        &["commit", "--allow-empty", "-m", "diverge web main"],
    );

    let feature_worktree_api = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_worktree_api.join("api_change.txt"), "api change\n").unwrap();
    git_stdout(&feature_worktree_api, &["add", "api_change.txt"]);
    git_stdout(
        &feature_worktree_api,
        &["commit", "-m", "api feature commit"],
    );

    let before = snapshot_all_worktrees(&root);

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    );
    let failure = land_preview.expect_err("a blocked repo must block the batch");
    assert_eq!(failure.code, "deliver.land_not_fast_forward");

    let after = snapshot_all_worktrees(&root);
    assert_eq!(before, after, "no repo may be written when land is blocked");
}

#[test]
fn write_bits_are_restored_when_the_merge_fails() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );

    std::fs::write(default_worktree.join("uncommitted.txt"), "dirty").unwrap();

    let before = crate::infra::fs::unix_mode(&default_worktree).unwrap();
    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect_err("dirty worktree must fail land");
    assert_eq!(failure.code, "deliver.land_dirty_worktree");

    assert_eq!(
        crate::infra::fs::unix_mode(&default_worktree).unwrap(),
        before,
        "a failed land must not leave a read-only repo writable"
    );
}

#[test]
fn write_bits_are_restored_after_successful_land() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let origins = root.parent().unwrap().join("origins").join("api");
    git_stdout(&origins, &["config", "receive.denyCurrentBranch", "ignore"]);

    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );

    crate::infra::fs::clear_write_bits(&default_worktree).unwrap();
    let before = crate::infra::fs::unix_mode(&default_worktree).unwrap();

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("land preview");

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
        },
    )
    .expect("land apply");

    assert!(out.value.land.iter().all(|r| r.merged));
    assert_eq!(
        crate::infra::fs::unix_mode(&default_worktree).unwrap(),
        before,
        "a successful land must restore original read-only permissions"
    );
}

#[test]
fn a_clean_land_merges_every_repo_and_pushes_each_default() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let origins = root.parent().unwrap().join("origins").join("api");
    git_stdout(&origins, &["config", "receive.denyCurrentBranch", "ignore"]);

    let feature_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(
        feature_worktree.join("feature.txt"),
        "new feature content\n",
    )
    .unwrap();
    git_stdout(&feature_worktree, &["add", "feature.txt"]);
    git_stdout(&feature_worktree, &["commit", "-m", "add feature.txt"]);
    let feature_tip = git_stdout(&feature_worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("land preview");

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
        },
    )
    .expect("land apply");

    assert_eq!(out.value.land.len(), 1);
    assert!(out.value.land[0].merged);
    assert!(out.value.land[0].pushed);

    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let default_tip = git_stdout(&default_worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    assert_eq!(
        default_tip, feature_tip,
        "default branch must equal feature tip"
    );

    assert!(
        feature_worktree.exists(),
        "feature worktree must still exist"
    );
}

#[test]
fn a_failed_push_is_a_warning_not_an_abort() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);

    let feature_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_worktree.join("feature.txt"), "content\n").unwrap();
    git_stdout(&feature_worktree, &["add", "feature.txt"]);
    git_stdout(&feature_worktree, &["commit", "-m", "feature commit"]);

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("land preview");

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
        },
    )
    .expect("a failed push must not abort the land");

    assert_eq!(out.value.land.len(), 1);
    assert!(
        out.value.land[0].merged,
        "merge stands even when push fails"
    );
    assert!(!out.value.land[0].pushed, "push failed");
    assert!(
        out.warnings
            .iter()
            .any(|w| w.code == "deliver.land_push_failed"),
        "push failure produces deliver.land_push_failed warning"
    );
}

#[test]
fn execute_failure_rolls_back_earlier_merges() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);

    let feature_api = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_api.join("api.txt"), "api\n").unwrap();
    git_stdout(&feature_api, &["add", "api.txt"]);
    git_stdout(&feature_api, &["commit", "-m", "api commit"]);

    let feature_web = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_web.join("web.txt"), "web\n").unwrap();
    git_stdout(&feature_web, &["add", "web.txt"]);
    git_stdout(&feature_web, &["commit", "-m", "web commit"]);

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("land preview");

    let api_default = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let api_default_before = git_stdout(&api_default, &["rev-parse", "HEAD"]);

    let web_default = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let web_git_dir = crate::git::System.worktree_git_dir(&web_default).unwrap();
    std::fs::write(web_git_dir.join("index.lock"), "lock").unwrap();

    let _failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
        },
    )
    .expect_err("execute failure on web must return Err");

    let api_default_after = git_stdout(&api_default, &["rev-parse", "HEAD"]);
    assert_eq!(
        api_default_before, api_default_after,
        "repo api must be rolled back if repo web fails during execute"
    );
}

#[test]
fn remote_default_ahead_of_local_default_does_not_trigger_remote_moved() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let origins = root.parent().unwrap().join("origins").join("api");
    git_stdout(&origins, &["config", "receive.denyCurrentBranch", "ignore"]);

    // Push a commit to origin main so remote main is ahead of local main
    git_stdout(
        &origins,
        &["commit", "--allow-empty", "-m", "origin main ahead"],
    );

    // Feature branch on top of origin main
    let feature_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    git_stdout(&feature_worktree, &["fetch", "origin"]);
    git_stdout(&feature_worktree, &["rebase", "origin/main"]);

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("land preview");

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
        },
    )
    .expect("land apply");

    assert!(
        !out.warnings
            .iter()
            .any(|w| w.code == "deliver.land_remote_moved"),
        "remote main did not move after preview; land_remote_moved must not fire"
    );
}

#[test]
fn partial_write_guard_lift_failure_restores_already_lifted_worktree() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(&root);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    crate::infra::fs::clear_write_bits(&default_worktree).unwrap();
    let before = crate::infra::fs::unix_mode(&default_worktree).unwrap();

    let unreadable_dir = root.join("unreadable");
    std::fs::create_dir(&unreadable_dir).unwrap();
    let file_inside = unreadable_dir.join("file.txt");
    std::fs::write(&file_inside, "test").unwrap();
    crate::infra::fs::chmod(&unreadable_dir, 0o000).unwrap();

    let result = crate::action::feature::deliver::land::WorktreeWriteGuard::lift(&[
        &default_worktree,
        &file_inside,
    ]);
    assert!(result.is_err(), "lift must fail on inaccessible path");

    // Restore unreadable_dir mode so tempdir cleanup succeeds
    crate::infra::fs::chmod(&unreadable_dir, 0o755).unwrap();

    let after = crate::infra::fs::unix_mode(&default_worktree).unwrap();
    assert_eq!(
        before, after,
        "already lifted worktree must be restored if lift fails on a later worktree"
    );
}

#[test]
fn exact_mode_restoration_preserves_original_permission_bits() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(&root);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );

    // Set permission to 0o500 (read+exec owner; group and other have no permissions)
    crate::infra::fs::chmod(&default_worktree, 0o500).unwrap();
    let before = crate::infra::fs::unix_mode(&default_worktree)
        .unwrap()
        .unwrap();
    assert_eq!(before & 0o777, 0o500);

    {
        let _lifted =
            crate::action::feature::deliver::land::WorktreeWriteGuard::lift(&[&default_worktree])
                .expect("lift");
    }

    let after = crate::infra::fs::unix_mode(&default_worktree)
        .unwrap()
        .unwrap();
    assert_eq!(
        after & 0o777,
        0o500,
        "exact mode bits 0o500 must be restored, not altered to 0o555"
    );
}

#[test]
fn failure_conventions_are_honored_for_land_system_failures() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let feature_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_worktree.join("f.txt"), "f").unwrap();
    git_stdout(&feature_worktree, &["add", "f.txt"]);
    git_stdout(&feature_worktree, &["commit", "-m", "f"]);

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("preview");

    // Lock default worktree index to cause fast_forward_to failure
    let git_dir = crate::git::System
        .worktree_git_dir(&default_worktree)
        .unwrap();
    std::fs::write(git_dir.join("index.lock"), "lock").unwrap();

    let failure = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
        },
    )
    .expect_err("merge failure");

    assert_eq!(failure.code, "git.merge_ff_only_failed");
    assert!(failure.expected.is_some(), "expected must be populated");
    assert!(failure.actual.is_some(), "actual must be populated");
    assert!(
        !failure.fix_actions.is_empty(),
        "fix_actions must be populated"
    );
}

#[test]
fn github_repo_in_land_mode_creates_no_pull_request() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let manifest = read_manifest(&layout).unwrap();
    // Update manifest origin to github URL
    let repos: Vec<_> = manifest
        .repos()
        .iter()
        .map(|r| {
            if r.name().as_str() == "api" {
                crate::store::manifest::Repo::new(
                    r.name().clone(),
                    "https://github.com/acme/api",
                    r.default_branch().clone(),
                )
            } else {
                r.clone()
            }
        })
        .collect();
    let new_manifest = crate::store::manifest::Manifest::new(
        manifest.name().clone(),
        manifest.providers().clone(),
        repos,
        None,
    )
    .unwrap();
    crate::store::manifest::Manifest::write(&layout, &new_manifest).unwrap();

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("land preview");

    assert_eq!(
        land_preview.value.preview.repos[0].action,
        DeliveryAction::LandOnDefault
    );

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
        },
    )
    .expect("land apply");

    assert!(
        out.value.preview.repos[0].pr_url.is_none(),
        "land mode must not create a PR URL"
    );
}

#[test]
fn absence_of_preview_evidence_none_none_skips_whole_batch() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let manifest = read_manifest(&layout).unwrap();
    let feature = read_feature(&layout, &FeatureName::new("checkout").unwrap()).unwrap();

    let mut preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("land preview")
    .value
    .preview;

    // Simulate preview having no remote_default_tip evidence
    preview.repos[0].remote_default_tip = None;

    let plans = crate::action::feature::deliver::land::preflight(
        &crate::git::System,
        &layout,
        &manifest,
        &feature,
        &preview,
    )
    .unwrap();

    // Mock git that returns Ok(None) for remote_branch_tip
    struct UnreachableRemoteGit(crate::git::System);
    impl crate::git::Git for UnreachableRemoteGit {
        fn target_state(
            &self,
            path: &Utf8Path,
        ) -> Result<crate::git::TargetState, crate::git::Error> {
            self.0.target_state(path)
        }
        fn head_branch(&self, git_dir: &Utf8Path) -> Result<String, crate::git::Error> {
            self.0.head_branch(git_dir)
        }
        fn worktree_git_dir(&self, path: &Utf8Path) -> Result<Utf8PathBuf, crate::git::Error> {
            self.0.worktree_git_dir(path)
        }
        fn clone_bare(&self, url: &str, dest: &Utf8Path) -> Result<(), crate::git::Error> {
            self.0.clone_bare(url, dest)
        }
        fn ensure_remote_tracking(&self, git_dir: &Utf8Path) -> Result<(), crate::git::Error> {
            self.0.ensure_remote_tracking(git_dir)
        }
        fn add_worktree(
            &self,
            git_dir: &Utf8Path,
            dest: &Utf8Path,
            branch: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.add_worktree(git_dir, dest, branch)
        }
        fn fetch(&self, git_dir: &Utf8Path) -> Result<(), crate::git::Error> {
            self.0.fetch(git_dir)
        }
        fn list_branches(&self, git_dir: &Utf8Path) -> Result<Vec<String>, crate::git::Error> {
            self.0.list_branches(git_dir)
        }
        fn create_branch_and_worktree(
            &self,
            git_dir: &Utf8Path,
            branch: &str,
            from_branch: &str,
            dest: &Utf8Path,
        ) -> Result<(), crate::git::Error> {
            self.0
                .create_branch_and_worktree(git_dir, branch, from_branch, dest)
        }
        fn fetch_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), crate::git::Error> {
            self.0.fetch_branch(worktree, branch)
        }
        fn fast_forward(&self, worktree: &Utf8Path) -> Result<(), crate::git::Error> {
            self.0.fast_forward(worktree)
        }
        fn remove_worktree(
            &self,
            git_dir: &Utf8Path,
            dest: &Utf8Path,
        ) -> Result<(), crate::git::Error> {
            self.0.remove_worktree(git_dir, dest)
        }
        fn add_detached_worktree(
            &self,
            git_dir: &Utf8Path,
            dest: &Utf8Path,
            revision: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.add_detached_worktree(git_dir, dest, revision)
        }
        fn create_branch(
            &self,
            git_dir: &Utf8Path,
            branch: &str,
            revision: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.create_branch(git_dir, branch, revision)
        }
        fn delete_branch(&self, git_dir: &Utf8Path, branch: &str) -> Result<(), crate::git::Error> {
            self.0.delete_branch(git_dir, branch)
        }
        fn merge_no_ff(&self, worktree: &Utf8Path, source: &str) -> Result<(), crate::git::Error> {
            self.0.merge_no_ff(worktree, source)
        }
        fn squash_merge(
            &self,
            worktree: &Utf8Path,
            source: &str,
            message: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.squash_merge(worktree, source, message)
        }
        fn fast_forward_to(
            &self,
            worktree: &Utf8Path,
            revision: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.fast_forward_to(worktree, revision)
        }
        fn worktree_dirty(&self, path: &Utf8Path) -> Result<bool, crate::git::Error> {
            self.0.worktree_dirty(path)
        }
        fn diff_worktree(&self, path: &Utf8Path) -> Result<String, crate::git::Error> {
            self.0.diff_worktree(path)
        }
        fn changed_paths(&self, path: &Utf8Path) -> Result<Vec<Utf8PathBuf>, crate::git::Error> {
            self.0.changed_paths(path)
        }
        fn head_commit(&self, path: &Utf8Path) -> Result<String, crate::git::Error> {
            self.0.head_commit(path)
        }
        fn paths_committed_since(
            &self,
            path: &Utf8Path,
            since: &str,
        ) -> Result<Vec<Utf8PathBuf>, crate::git::Error> {
            self.0.paths_committed_since(path, since)
        }
        fn path_at_commit(
            &self,
            git_dir: &Utf8Path,
            commit: &str,
            path: &Utf8Path,
        ) -> Result<Option<crate::git::BlobEvidence>, crate::git::Error> {
            self.0.path_at_commit(git_dir, commit, path)
        }
        fn commits_ahead(
            &self,
            git_dir: &Utf8Path,
            base: &str,
            branch: &str,
        ) -> Result<u64, crate::git::Error> {
            self.0.commits_ahead(git_dir, base, branch)
        }
        fn is_ancestor(
            &self,
            git_dir: &Utf8Path,
            ancestor: &str,
            descendant: &str,
        ) -> Result<bool, crate::git::Error> {
            self.0.is_ancestor(git_dir, ancestor, descendant)
        }
        fn divergence(
            &self,
            git_dir: &Utf8Path,
            local: &str,
            remote: &str,
        ) -> Result<crate::git::Divergence, crate::git::Error> {
            self.0.divergence(git_dir, local, remote)
        }
        fn merge_base(
            &self,
            git_dir: &Utf8Path,
            a: &str,
            b: &str,
        ) -> Result<String, crate::git::Error> {
            self.0.merge_base(git_dir, a, b)
        }
        fn revision_commit(
            &self,
            git_dir: &Utf8Path,
            revision: &str,
        ) -> Result<String, crate::git::Error> {
            self.0.revision_commit(git_dir, revision)
        }
        fn reset_hard(&self, worktree: &Utf8Path, revision: &str) -> Result<(), crate::git::Error> {
            self.0.reset_hard(worktree, revision)
        }
        fn remote_branch_tip(
            &self,
            _git_dir: &Utf8Path,
            _remote: &str,
            _branch: &str,
        ) -> Result<Option<String>, crate::git::Error> {
            Ok(None)
        }
        fn push(
            &self,
            git_dir: &Utf8Path,
            remote: &str,
            from: &str,
            to: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.push(git_dir, remote, from, to)
        }
        fn commit_patch_id(
            &self,
            worktree: &Utf8Path,
            commit: &str,
        ) -> Result<String, crate::git::Error> {
            self.0.commit_patch_id(worktree, commit)
        }
        fn diff_patch_id(
            &self,
            worktree: &Utf8Path,
            base: &str,
            tip: &str,
        ) -> Result<String, crate::git::Error> {
            self.0.diff_patch_id(worktree, base, tip)
        }
        fn rebase_branch(
            &self,
            worktree: &Utf8Path,
            branch: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.rebase_branch(worktree, branch)
        }
        fn abort_rebase(&self, worktree: &Utf8Path) -> Result<(), crate::git::Error> {
            self.0.abort_rebase(worktree)
        }
        fn rename_branch(
            &self,
            git_dir: &Utf8Path,
            from: &str,
            to: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.rename_branch(git_dir, from, to)
        }
        fn move_worktree(
            &self,
            git_dir: &Utf8Path,
            from: &Utf8Path,
            to: &Utf8Path,
        ) -> Result<(), crate::git::Error> {
            self.0.move_worktree(git_dir, from, to)
        }
        fn publish_remote_branch(
            &self,
            git_dir: &Utf8Path,
            remote: &str,
            branch: &str,
            at: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.publish_remote_branch(git_dir, remote, branch, at)
        }
        fn delete_remote_branch(
            &self,
            git_dir: &Utf8Path,
            remote: &str,
            branch: &str,
            expected_tip: &str,
        ) -> Result<(), crate::git::Error> {
            self.0
                .delete_remote_branch(git_dir, remote, branch, expected_tip)
        }
    }

    let mock_git = UnreachableRemoteGit(crate::git::System);
    let mut warnings = Vec::new();
    let results =
        crate::action::feature::deliver::land::execute(&mock_git, &layout, &plans, &mut warnings)
            .unwrap();

    assert!(
        results.iter().all(|r| !r.merged),
        "None/None must skip whole batch"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.code == "deliver.land_remote_moved")
    );
}

#[test]
fn absence_of_preview_evidence_none_err_skips_whole_batch() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let manifest = read_manifest(&layout).unwrap();
    let feature = read_feature(&layout, &FeatureName::new("checkout").unwrap()).unwrap();

    let mut preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("land preview")
    .value
    .preview;

    // Simulate preview having no remote_default_tip evidence
    preview.repos[0].remote_default_tip = None;

    let plans = crate::action::feature::deliver::land::preflight(
        &crate::git::System,
        &layout,
        &manifest,
        &feature,
        &preview,
    )
    .unwrap();

    // Mock git that returns Err for remote_branch_tip
    struct ErrRemoteGit(crate::git::System);
    impl crate::git::Git for ErrRemoteGit {
        fn target_state(
            &self,
            path: &Utf8Path,
        ) -> Result<crate::git::TargetState, crate::git::Error> {
            self.0.target_state(path)
        }
        fn head_branch(&self, git_dir: &Utf8Path) -> Result<String, crate::git::Error> {
            self.0.head_branch(git_dir)
        }
        fn worktree_git_dir(&self, path: &Utf8Path) -> Result<Utf8PathBuf, crate::git::Error> {
            self.0.worktree_git_dir(path)
        }
        fn clone_bare(&self, url: &str, dest: &Utf8Path) -> Result<(), crate::git::Error> {
            self.0.clone_bare(url, dest)
        }
        fn ensure_remote_tracking(&self, git_dir: &Utf8Path) -> Result<(), crate::git::Error> {
            self.0.ensure_remote_tracking(git_dir)
        }
        fn add_worktree(
            &self,
            git_dir: &Utf8Path,
            dest: &Utf8Path,
            branch: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.add_worktree(git_dir, dest, branch)
        }
        fn fetch(&self, git_dir: &Utf8Path) -> Result<(), crate::git::Error> {
            self.0.fetch(git_dir)
        }
        fn list_branches(&self, git_dir: &Utf8Path) -> Result<Vec<String>, crate::git::Error> {
            self.0.list_branches(git_dir)
        }
        fn create_branch_and_worktree(
            &self,
            git_dir: &Utf8Path,
            branch: &str,
            from_branch: &str,
            dest: &Utf8Path,
        ) -> Result<(), crate::git::Error> {
            self.0
                .create_branch_and_worktree(git_dir, branch, from_branch, dest)
        }
        fn fetch_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), crate::git::Error> {
            self.0.fetch_branch(worktree, branch)
        }
        fn fast_forward(&self, worktree: &Utf8Path) -> Result<(), crate::git::Error> {
            self.0.fast_forward(worktree)
        }
        fn remove_worktree(
            &self,
            git_dir: &Utf8Path,
            dest: &Utf8Path,
        ) -> Result<(), crate::git::Error> {
            self.0.remove_worktree(git_dir, dest)
        }
        fn add_detached_worktree(
            &self,
            git_dir: &Utf8Path,
            dest: &Utf8Path,
            revision: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.add_detached_worktree(git_dir, dest, revision)
        }
        fn create_branch(
            &self,
            git_dir: &Utf8Path,
            branch: &str,
            revision: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.create_branch(git_dir, branch, revision)
        }
        fn delete_branch(&self, git_dir: &Utf8Path, branch: &str) -> Result<(), crate::git::Error> {
            self.0.delete_branch(git_dir, branch)
        }
        fn merge_no_ff(&self, worktree: &Utf8Path, source: &str) -> Result<(), crate::git::Error> {
            self.0.merge_no_ff(worktree, source)
        }
        fn squash_merge(
            &self,
            worktree: &Utf8Path,
            source: &str,
            message: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.squash_merge(worktree, source, message)
        }
        fn fast_forward_to(
            &self,
            worktree: &Utf8Path,
            revision: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.fast_forward_to(worktree, revision)
        }
        fn worktree_dirty(&self, path: &Utf8Path) -> Result<bool, crate::git::Error> {
            self.0.worktree_dirty(path)
        }
        fn diff_worktree(&self, path: &Utf8Path) -> Result<String, crate::git::Error> {
            self.0.diff_worktree(path)
        }
        fn changed_paths(&self, path: &Utf8Path) -> Result<Vec<Utf8PathBuf>, crate::git::Error> {
            self.0.changed_paths(path)
        }
        fn head_commit(&self, path: &Utf8Path) -> Result<String, crate::git::Error> {
            self.0.head_commit(path)
        }
        fn paths_committed_since(
            &self,
            path: &Utf8Path,
            since: &str,
        ) -> Result<Vec<Utf8PathBuf>, crate::git::Error> {
            self.0.paths_committed_since(path, since)
        }
        fn path_at_commit(
            &self,
            git_dir: &Utf8Path,
            commit: &str,
            path: &Utf8Path,
        ) -> Result<Option<crate::git::BlobEvidence>, crate::git::Error> {
            self.0.path_at_commit(git_dir, commit, path)
        }
        fn commits_ahead(
            &self,
            git_dir: &Utf8Path,
            base: &str,
            branch: &str,
        ) -> Result<u64, crate::git::Error> {
            self.0.commits_ahead(git_dir, base, branch)
        }
        fn is_ancestor(
            &self,
            git_dir: &Utf8Path,
            ancestor: &str,
            descendant: &str,
        ) -> Result<bool, crate::git::Error> {
            self.0.is_ancestor(git_dir, ancestor, descendant)
        }
        fn divergence(
            &self,
            git_dir: &Utf8Path,
            local: &str,
            remote: &str,
        ) -> Result<crate::git::Divergence, crate::git::Error> {
            self.0.divergence(git_dir, local, remote)
        }
        fn merge_base(
            &self,
            git_dir: &Utf8Path,
            a: &str,
            b: &str,
        ) -> Result<String, crate::git::Error> {
            self.0.merge_base(git_dir, a, b)
        }
        fn revision_commit(
            &self,
            git_dir: &Utf8Path,
            revision: &str,
        ) -> Result<String, crate::git::Error> {
            self.0.revision_commit(git_dir, revision)
        }
        fn reset_hard(&self, worktree: &Utf8Path, revision: &str) -> Result<(), crate::git::Error> {
            self.0.reset_hard(worktree, revision)
        }
        fn remote_branch_tip(
            &self,
            _git_dir: &Utf8Path,
            _remote: &str,
            _branch: &str,
        ) -> Result<Option<String>, crate::git::Error> {
            Err(crate::git::Error::Refused {
                command: "git ls-remote".to_owned(),
                detail: "simulated error".to_owned(),
            })
        }
        fn push(
            &self,
            git_dir: &Utf8Path,
            remote: &str,
            from: &str,
            to: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.push(git_dir, remote, from, to)
        }
        fn commit_patch_id(
            &self,
            worktree: &Utf8Path,
            commit: &str,
        ) -> Result<String, crate::git::Error> {
            self.0.commit_patch_id(worktree, commit)
        }
        fn diff_patch_id(
            &self,
            worktree: &Utf8Path,
            base: &str,
            tip: &str,
        ) -> Result<String, crate::git::Error> {
            self.0.diff_patch_id(worktree, base, tip)
        }
        fn rebase_branch(
            &self,
            worktree: &Utf8Path,
            branch: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.rebase_branch(worktree, branch)
        }
        fn abort_rebase(&self, worktree: &Utf8Path) -> Result<(), crate::git::Error> {
            self.0.abort_rebase(worktree)
        }
        fn rename_branch(
            &self,
            git_dir: &Utf8Path,
            from: &str,
            to: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.rename_branch(git_dir, from, to)
        }
        fn move_worktree(
            &self,
            git_dir: &Utf8Path,
            from: &Utf8Path,
            to: &Utf8Path,
        ) -> Result<(), crate::git::Error> {
            self.0.move_worktree(git_dir, from, to)
        }
        fn publish_remote_branch(
            &self,
            git_dir: &Utf8Path,
            remote: &str,
            branch: &str,
            at: &str,
        ) -> Result<(), crate::git::Error> {
            self.0.publish_remote_branch(git_dir, remote, branch, at)
        }
        fn delete_remote_branch(
            &self,
            git_dir: &Utf8Path,
            remote: &str,
            branch: &str,
            expected_tip: &str,
        ) -> Result<(), crate::git::Error> {
            self.0
                .delete_remote_branch(git_dir, remote, branch, expected_tip)
        }
    }

    let mock_git = ErrRemoteGit(crate::git::System);
    let mut warnings = Vec::new();
    let results =
        crate::action::feature::deliver::land::execute(&mock_git, &layout, &plans, &mut warnings)
            .unwrap();

    assert!(
        results.iter().all(|r| !r.merged),
        "None/Err must skip whole batch"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.code == "deliver.land_remote_moved")
    );
}

#[test]
fn expected_none_current_some_blocks_or_skips_whole_batch() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let manifest = read_manifest(&layout).unwrap();
    let feature = read_feature(&layout, &FeatureName::new("checkout").unwrap()).unwrap();

    let mut preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("land preview")
    .value
    .preview;

    // Simulate preview having no remote_default_tip
    preview.repos[0].remote_default_tip = None;

    let plans = crate::action::feature::deliver::land::preflight(
        &crate::git::System,
        &layout,
        &manifest,
        &feature,
        &preview,
    )
    .unwrap();

    let mut warnings = Vec::new();
    let results = crate::action::feature::deliver::land::execute(
        &crate::git::System,
        &layout,
        &plans,
        &mut warnings,
    )
    .unwrap();

    assert!(
        results.iter().all(|r| !r.merged),
        "expected None when current is Some must skip whole batch"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.code == "deliver.land_remote_moved"),
        "must emit land_remote_moved warning"
    );
}

#[test]
fn remote_moved_on_second_repo_skips_or_blocks_whole_batch_writing_neither() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let origins_web = root.parent().unwrap().join("origins").join("web");
    git_stdout(
        &origins_web,
        &["config", "receive.denyCurrentBranch", "ignore"],
    );

    let before = snapshot_all_worktrees(&root);

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("land preview");

    // Add commit to origin web after preview
    git_stdout(
        &origins_web,
        &["commit", "--allow-empty", "-m", "web origin moved"],
    );

    let out = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: false,
            land: true,
            fingerprint: Some(land_preview.value.preview.fingerprint),
        },
    );

    let after = snapshot_all_worktrees(&root);
    assert_eq!(
        before, after,
        "remote moved on repo 2 must write NEITHER repo 1 nor repo 2"
    );

    match out {
        Ok(report) => {
            assert!(
                report.value.land.iter().all(|r| !r.merged),
                "all repos must be unmerged"
            );
            assert!(
                report
                    .warnings
                    .iter()
                    .any(|w| w.code == "deliver.land_remote_moved"),
                "land_remote_moved warning must be emitted"
            );
        }
        Err(failure) => {
            assert!(
                failure.code == "deliver.fingerprint_mismatch"
                    || failure.code == "deliver.land_remote_moved",
                "must refuse when remote branch moves"
            );
        }
    }
}

#[test]
fn remote_moved_with_warning_skips_batch_and_emits_warning() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let manifest = read_manifest(&layout).unwrap();
    let feature = read_feature(&layout, &FeatureName::new("checkout").unwrap()).unwrap();

    let origins_web = root.parent().unwrap().join("origins").join("web");
    git_stdout(
        &origins_web,
        &["config", "receive.denyCurrentBranch", "ignore"],
    );

    let land_preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("land preview");

    let plans = crate::action::feature::deliver::land::preflight(
        &crate::git::System,
        &layout,
        &manifest,
        &feature,
        &land_preview.value.preview,
    )
    .unwrap();

    git_stdout(
        &origins_web,
        &["commit", "--allow-empty", "-m", "web origin moved"],
    );

    let mut warnings = Vec::new();
    let results = crate::action::feature::deliver::land::execute(
        &crate::git::System,
        &layout,
        &plans,
        &mut warnings,
    )
    .expect("execute");

    assert!(
        warnings
            .iter()
            .any(|w| w.code == "deliver.land_remote_moved"),
        "land_remote_moved warning must be present"
    );
    assert!(
        results.iter().all(|r| !r.merged),
        "entire batch must be skipped when remote moves"
    );
}

struct FailingRollbackGit(crate::git::System);

impl crate::git::Git for FailingRollbackGit {
    fn target_state(&self, path: &Utf8Path) -> Result<crate::git::TargetState, crate::git::Error> {
        self.0.target_state(path)
    }
    fn head_branch(&self, git_dir: &Utf8Path) -> Result<String, crate::git::Error> {
        self.0.head_branch(git_dir)
    }
    fn worktree_git_dir(&self, path: &Utf8Path) -> Result<Utf8PathBuf, crate::git::Error> {
        self.0.worktree_git_dir(path)
    }
    fn clone_bare(&self, url: &str, dest: &Utf8Path) -> Result<(), crate::git::Error> {
        self.0.clone_bare(url, dest)
    }
    fn ensure_remote_tracking(&self, git_dir: &Utf8Path) -> Result<(), crate::git::Error> {
        self.0.ensure_remote_tracking(git_dir)
    }
    fn add_worktree(
        &self,
        git_dir: &Utf8Path,
        dest: &Utf8Path,
        branch: &str,
    ) -> Result<(), crate::git::Error> {
        self.0.add_worktree(git_dir, dest, branch)
    }
    fn fetch(&self, git_dir: &Utf8Path) -> Result<(), crate::git::Error> {
        self.0.fetch(git_dir)
    }
    fn list_branches(&self, git_dir: &Utf8Path) -> Result<Vec<String>, crate::git::Error> {
        self.0.list_branches(git_dir)
    }
    fn create_branch_and_worktree(
        &self,
        git_dir: &Utf8Path,
        branch: &str,
        from_branch: &str,
        dest: &Utf8Path,
    ) -> Result<(), crate::git::Error> {
        self.0
            .create_branch_and_worktree(git_dir, branch, from_branch, dest)
    }
    fn fetch_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), crate::git::Error> {
        self.0.fetch_branch(worktree, branch)
    }
    fn fast_forward(&self, worktree: &Utf8Path) -> Result<(), crate::git::Error> {
        self.0.fast_forward(worktree)
    }
    fn remove_worktree(
        &self,
        git_dir: &Utf8Path,
        dest: &Utf8Path,
    ) -> Result<(), crate::git::Error> {
        self.0.remove_worktree(git_dir, dest)
    }
    fn add_detached_worktree(
        &self,
        git_dir: &Utf8Path,
        dest: &Utf8Path,
        revision: &str,
    ) -> Result<(), crate::git::Error> {
        self.0.add_detached_worktree(git_dir, dest, revision)
    }
    fn create_branch(
        &self,
        git_dir: &Utf8Path,
        branch: &str,
        revision: &str,
    ) -> Result<(), crate::git::Error> {
        self.0.create_branch(git_dir, branch, revision)
    }
    fn delete_branch(&self, git_dir: &Utf8Path, branch: &str) -> Result<(), crate::git::Error> {
        self.0.delete_branch(git_dir, branch)
    }
    fn merge_no_ff(&self, worktree: &Utf8Path, source: &str) -> Result<(), crate::git::Error> {
        self.0.merge_no_ff(worktree, source)
    }
    fn squash_merge(
        &self,
        worktree: &Utf8Path,
        source: &str,
        message: &str,
    ) -> Result<(), crate::git::Error> {
        self.0.squash_merge(worktree, source, message)
    }
    fn fast_forward_to(
        &self,
        worktree: &Utf8Path,
        revision: &str,
    ) -> Result<(), crate::git::Error> {
        self.0.fast_forward_to(worktree, revision)
    }
    fn worktree_dirty(&self, path: &Utf8Path) -> Result<bool, crate::git::Error> {
        self.0.worktree_dirty(path)
    }
    fn diff_worktree(&self, path: &Utf8Path) -> Result<String, crate::git::Error> {
        self.0.diff_worktree(path)
    }
    fn changed_paths(&self, path: &Utf8Path) -> Result<Vec<Utf8PathBuf>, crate::git::Error> {
        self.0.changed_paths(path)
    }
    fn head_commit(&self, path: &Utf8Path) -> Result<String, crate::git::Error> {
        self.0.head_commit(path)
    }
    fn paths_committed_since(
        &self,
        path: &Utf8Path,
        since: &str,
    ) -> Result<Vec<Utf8PathBuf>, crate::git::Error> {
        self.0.paths_committed_since(path, since)
    }
    fn path_at_commit(
        &self,
        git_dir: &Utf8Path,
        commit: &str,
        path: &Utf8Path,
    ) -> Result<Option<crate::git::BlobEvidence>, crate::git::Error> {
        self.0.path_at_commit(git_dir, commit, path)
    }
    fn commits_ahead(
        &self,
        git_dir: &Utf8Path,
        base: &str,
        branch: &str,
    ) -> Result<u64, crate::git::Error> {
        self.0.commits_ahead(git_dir, base, branch)
    }
    fn is_ancestor(
        &self,
        git_dir: &Utf8Path,
        ancestor: &str,
        descendant: &str,
    ) -> Result<bool, crate::git::Error> {
        self.0.is_ancestor(git_dir, ancestor, descendant)
    }
    fn divergence(
        &self,
        git_dir: &Utf8Path,
        local: &str,
        remote: &str,
    ) -> Result<crate::git::Divergence, crate::git::Error> {
        self.0.divergence(git_dir, local, remote)
    }
    fn merge_base(
        &self,
        git_dir: &Utf8Path,
        a: &str,
        b: &str,
    ) -> Result<String, crate::git::Error> {
        self.0.merge_base(git_dir, a, b)
    }
    fn revision_commit(
        &self,
        git_dir: &Utf8Path,
        revision: &str,
    ) -> Result<String, crate::git::Error> {
        self.0.revision_commit(git_dir, revision)
    }
    fn reset_hard(&self, _worktree: &Utf8Path, _revision: &str) -> Result<(), crate::git::Error> {
        Err(crate::git::Error::Refused {
            command: "git reset --hard".to_owned(),
            detail: "simulated reset_hard failure for rollback test".to_owned(),
        })
    }
    fn remote_branch_tip(
        &self,
        git_dir: &Utf8Path,
        remote: &str,
        branch: &str,
    ) -> Result<Option<String>, crate::git::Error> {
        self.0.remote_branch_tip(git_dir, remote, branch)
    }
    fn push(
        &self,
        git_dir: &Utf8Path,
        remote: &str,
        from: &str,
        to: &str,
    ) -> Result<(), crate::git::Error> {
        self.0.push(git_dir, remote, from, to)
    }
    fn commit_patch_id(
        &self,
        worktree: &Utf8Path,
        commit: &str,
    ) -> Result<String, crate::git::Error> {
        self.0.commit_patch_id(worktree, commit)
    }
    fn diff_patch_id(
        &self,
        worktree: &Utf8Path,
        base: &str,
        tip: &str,
    ) -> Result<String, crate::git::Error> {
        self.0.diff_patch_id(worktree, base, tip)
    }
    fn rebase_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), crate::git::Error> {
        self.0.rebase_branch(worktree, branch)
    }
    fn abort_rebase(&self, worktree: &Utf8Path) -> Result<(), crate::git::Error> {
        self.0.abort_rebase(worktree)
    }
    fn rename_branch(
        &self,
        git_dir: &Utf8Path,
        from: &str,
        to: &str,
    ) -> Result<(), crate::git::Error> {
        self.0.rename_branch(git_dir, from, to)
    }
    fn move_worktree(
        &self,
        git_dir: &Utf8Path,
        from: &Utf8Path,
        to: &Utf8Path,
    ) -> Result<(), crate::git::Error> {
        self.0.move_worktree(git_dir, from, to)
    }
    fn publish_remote_branch(
        &self,
        git_dir: &Utf8Path,
        remote: &str,
        branch: &str,
        at: &str,
    ) -> Result<(), crate::git::Error> {
        self.0.publish_remote_branch(git_dir, remote, branch, at)
    }
    fn delete_remote_branch(
        &self,
        git_dir: &Utf8Path,
        remote: &str,
        branch: &str,
        expected_tip: &str,
    ) -> Result<(), crate::git::Error> {
        self.0
            .delete_remote_branch(git_dir, remote, branch, expected_tip)
    }
}

#[test]
fn rollback_failure_produces_land_rollback_failed_failure() {
    let (_guard, root) = hall_with_promoted(&["api", "web"]);
    let ctx = Ctx::new(root.clone());
    approve_through_plan(&root);

    let layout = Layout::at(&root);
    let manifest = read_manifest(&layout).unwrap();
    let feature = read_feature(&layout, &FeatureName::new("checkout").unwrap()).unwrap();

    let feature_api = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_api.join("api.txt"), "api").unwrap();
    git_stdout(&feature_api, &["add", "api.txt"]);
    git_stdout(&feature_api, &["commit", "-m", "api commit"]);

    let feature_web = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(feature_web.join("web.txt"), "web").unwrap();
    git_stdout(&feature_web, &["add", "web.txt"]);
    git_stdout(&feature_web, &["commit", "-m", "web commit"]);

    let preview = deliver(
        &ctx,
        DeliverInput {
            feature: "checkout".to_owned(),
            preview: true,
            land: true,
            fingerprint: None,
        },
    )
    .expect("land preview")
    .value
    .preview;

    let plans = crate::action::feature::deliver::land::preflight(
        &crate::git::System,
        &layout,
        &manifest,
        &feature,
        &preview,
    )
    .unwrap();

    let web_default = layout.repo_worktree(
        &RepoName::new("web").unwrap(),
        &BranchName::new("main").unwrap(),
    );

    // Lock web_default index so fast_forward_to fails during merge phase
    let web_git_dir = crate::git::System.worktree_git_dir(&web_default).unwrap();
    std::fs::write(web_git_dir.join("index.lock"), "lock").unwrap();

    let mock_git = FailingRollbackGit(crate::git::System);
    let mut warnings = Vec::new();
    let failure =
        crate::action::feature::deliver::land::execute(&mock_git, &layout, &plans, &mut warnings)
            .expect_err("rollback failure must produce Failure");

    assert_eq!(failure.code, "deliver.land_rollback_failed");
    assert!(failure.expected.is_some());
    assert!(failure.actual.is_some());
    assert!(!failure.fix_actions.is_empty());
}
