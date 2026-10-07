//! Unit tests for `crate::action::feature::relations` — the child-derived
//! tree projection, receipt freshness, and descendant blockers.
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
use crate::action::feature::verification;
use crate::domain::feature::{
    FeatureIntegrationState, IntegrationReceipt, IntegrationStrategy, IntegrationVia,
    VerificationEvidence, VerificationResult,
};
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::test_support::{git, hall_root, seeded_repo};

/// Write a v2 manifest declaring one repo, `api`, pointing at `url`, with the
/// given ordered checks.
fn write_manifest(layout: &Layout, url: &str, checks: &[&str]) {
    let checks: Vec<serde_json::Value> = checks
        .iter()
        .map(|check| serde_json::json!(check))
        .collect();
    let manifest = serde_json::json!({
        "version": 2,
        "name": "acme",
        "integration": { "via": "local", "strategy": "squash" },
        "providers": { "available": ["claude-code"], "default": "claude-code" },
        "repos": [
            { "name": "api", "url": url, "default_branch": "main", "checks": checks }
        ],
    });
    fs::write_text(
        &layout.manifest(),
        &serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();
}

/// A hall with a real bare clone of `api` on `main`, and branches `parent`
/// and `child` cut from it. Returns (guard, layout).
fn seeded_relation_hall() -> (tempfile::TempDir, Layout) {
    let (guard, root) = hall_root();
    let layout = Layout::at(&root);

    let origin = seeded_repo(&root.join("origin"), "main");
    write_manifest(&layout, origin.as_str(), &["true"]);

    let bare = layout.repo_bare(&RepoName::new("api").unwrap());
    crate::git::System
        .clone_bare(origin.as_str(), &bare)
        .unwrap();
    git(&bare, &["branch", "parent"]);
    git(&bare, &["branch", "child"]);

    (guard, layout)
}

/// Commit `content` to `file` on `branch` in the api bare clone, returning
/// the new tip SHA. The worktree is left in place (features retain worktrees).
fn commit_on_child(layout: &Layout, branch: &str, file: &str, content: &str) -> String {
    let api = RepoName::new("api").unwrap();
    let bare = layout.repo_bare(&api);
    let branch_name = BranchName::new(branch).unwrap();
    let worktree = layout.repo_worktree(&api, &branch_name);
    if crate::git::System.target_state(&worktree).unwrap() != crate::git::TargetState::Repository {
        crate::git::System
            .add_worktree(&bare, &worktree, branch)
            .unwrap();
    }
    std::fs::write(worktree.join(file), content).unwrap();
    git(&worktree, &["add", file]);
    git(
        &worktree,
        &["commit", "-m", format!("{file}: {content}").as_str()],
    );
    crate::git::System.head_commit(&worktree).unwrap()
}

fn feature(layout: &Layout, name: &str) -> Feature {
    read_feature(layout, &FeatureName::new(name).unwrap()).unwrap()
}

/// Write a feature with `name`, optional `parent`, and a promotion on `api`
/// carrying `receipt` (when given). Branches are named after the feature.
fn write_feature(
    layout: &Layout,
    name: &str,
    parent: Option<&str>,
    receipt: Option<IntegrationReceipt>,
) {
    let mut feature = Feature::new(
        FeatureName::new(name).unwrap(),
        BranchName::new(name).unwrap(),
    );
    feature.parent = parent.map(|p| FeatureName::new(p).unwrap());
    feature.promote(RepoName::new("api").unwrap());
    if let Some(receipt) = receipt {
        feature
            .promotions
            .get_mut(&RepoName::new("api").unwrap())
            .unwrap()
            .integration_receipt = Some(receipt);
    }
    feature.write(layout).unwrap();
}

fn passing_receipt(source_sha: &str, result_sha: &str) -> IntegrationReceipt {
    IntegrationReceipt {
        source_sha: source_sha.to_owned(),
        target_branch: BranchName::new("parent").unwrap(),
        result_sha: result_sha.to_owned(),
        via: IntegrationVia::Local,
        strategy: IntegrationStrategy::Squash,
        pr_url: None,
        verification: VerificationEvidence {
            command_fingerprint: verification::fingerprint(&["true".to_owned()]).unwrap(),
            child: vec![VerificationResult::passed("true", Some(0), "")],
            parent: vec![VerificationResult::passed("true", Some(0), "")],
            pr_checks: Vec::new(),
            verified_at: "2026-08-14T12:00:00Z".to_owned(),
        },
    }
}

// -- tree projection -------------------------------------------------------

#[test]
fn children_are_inferred_and_the_parent_chain_is_unlimited() {
    let (_guard, layout) = hall_root();
    let layout = Layout::at(&layout);
    write_feature(&layout, "root", None, None);
    write_feature(&layout, "parent", Some("root"), None);
    write_feature(&layout, "child", Some("parent"), None);
    write_feature(&layout, "leaf", Some("child"), None);

    // Only the child stores the edge; nobody stores a child list.
    assert_eq!(
        feature(&layout, "child")
            .parent
            .as_ref()
            .map(|n| n.as_str()),
        Some("parent")
    );
    assert_eq!(feature(&layout, "root").parent, None);

    let root_descendants = descendants(&layout, &FeatureName::new("root").unwrap()).unwrap();
    let names: Vec<&str> = root_descendants.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["parent", "child", "leaf"]);

    let parent_descendants = descendants(&layout, &FeatureName::new("parent").unwrap()).unwrap();
    let names: Vec<&str> = parent_descendants.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["child", "leaf"]);

    assert!(
        descendants(&layout, &FeatureName::new("leaf").unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn descendants_are_deterministic_pre_order_sorted_by_name() {
    let (_guard, layout) = hall_root();
    let layout = Layout::at(&layout);
    write_feature(&layout, "root", None, None);
    write_feature(&layout, "zeta", Some("root"), None);
    write_feature(&layout, "alpha", Some("root"), None);
    write_feature(&layout, "zeta-one", Some("zeta"), None);

    let all = descendants(&layout, &FeatureName::new("root").unwrap()).unwrap();
    let names: Vec<&str> = all.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["alpha", "zeta", "zeta-one"]);
}

#[test]
fn a_missing_parent_is_a_hard_refusal() {
    let (_guard, layout) = hall_root();
    let layout = Layout::at(&layout);
    write_feature(&layout, "orphan", Some("ghost"), None);

    let error = read_all(&layout).unwrap_err();
    assert_eq!(error.code, "feature.parent_missing");
    assert!(error.what.contains("ghost"));
}

#[test]
fn a_hand_edited_cycle_is_a_hard_refusal() {
    let (_guard, layout) = hall_root();
    let layout = Layout::at(&layout);
    write_feature(&layout, "a", Some("b"), None);
    write_feature(&layout, "b", Some("a"), None);

    let error = read_all(&layout).unwrap_err();
    assert_eq!(error.code, "feature.parent_cycle");
}

#[test]
fn read_all_returns_every_feature_sorted_and_read_feature_finds_one() {
    let (_guard, layout) = hall_root();
    let layout = Layout::at(&layout);
    write_feature(&layout, "zeta", None, None);
    write_feature(&layout, "alpha", None, None);

    let all = read_all(&layout).unwrap();
    let names: Vec<&str> = all.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["alpha", "zeta"]);

    let error = read_feature(&layout, &FeatureName::new("ghost").unwrap()).unwrap_err();
    assert_eq!(error.code, "feature.not_found");
}

#[test]
fn parent_reads_the_immediate_parent_feature() {
    let (_guard, layout) = hall_root();
    let layout = Layout::at(&layout);
    write_feature(&layout, "root", None, None);
    write_feature(&layout, "child", Some("root"), None);

    assert_eq!(parent(&layout, &feature(&layout, "root")).unwrap(), None);
    let parent_feature = parent(&layout, &feature(&layout, "child"))
        .unwrap()
        .unwrap();
    assert_eq!(parent_feature.name.as_str(), "root");

    // A parent that vanishes after being referenced is a hard refusal.
    fs::remove_path(&layout.feature_dir(&FeatureName::new("root").unwrap())).unwrap();
    let error = parent(&layout, &feature(&layout, "child")).unwrap_err();
    assert_eq!(error.code, "feature.parent_missing");
}

// -- blockers and freshness (real git) -------------------------------------

#[test]
fn a_leaf_has_no_descendant_blockers() {
    let (_guard, layout) = seeded_relation_hall();
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    write_feature(&layout, "parent", None, None);
    write_feature(&layout, "leaf", Some("parent"), None);

    let blockers = blocking_descendants(
        &crate::git::System,
        &layout,
        &manifest,
        &feature(&layout, "leaf"),
    )
    .unwrap();
    assert!(blockers.is_empty());
}

#[test]
fn a_child_is_blocked_by_an_active_leaf() {
    let (_guard, layout) = seeded_relation_hall();
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    write_feature(&layout, "parent", None, None);
    write_feature(&layout, "child", Some("parent"), None);
    write_feature(&layout, "leaf", Some("child"), None);

    let blockers = blocking_descendants(
        &crate::git::System,
        &layout,
        &manifest,
        &feature(&layout, "child"),
    )
    .unwrap();
    let names: Vec<&str> = blockers
        .iter()
        .map(|entry| entry.feature.as_str())
        .collect();
    assert_eq!(names, ["leaf"]);
    assert_eq!(blockers[0].depth, 1);
    assert_eq!(blockers[0].state, FeatureIntegrationState::Active);
}

#[test]
fn a_root_is_blocked_by_descendants_at_any_depth() {
    let (_guard, layout) = seeded_relation_hall();
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    write_feature(&layout, "root", None, None);
    write_feature(&layout, "parent", Some("root"), None);
    write_feature(&layout, "child", Some("parent"), None);
    write_feature(&layout, "leaf", Some("child"), None);

    let blockers = blocking_descendants(
        &crate::git::System,
        &layout,
        &manifest,
        &feature(&layout, "root"),
    )
    .unwrap();
    let names: Vec<&str> = blockers
        .iter()
        .map(|entry| entry.feature.as_str())
        .collect();
    assert_eq!(names, ["parent", "child", "leaf"]);
}

#[test]
fn an_abandoned_node_does_not_block_but_its_active_child_does() {
    let (_guard, layout) = seeded_relation_hall();
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    write_feature(&layout, "root", None, None);
    write_feature(&layout, "abandoned", Some("root"), None);
    write_feature(&layout, "active", Some("abandoned"), None);

    // Close the abandoned node as abandoned — history, not a blocker.
    let name = FeatureName::new("abandoned").unwrap();
    crate::action::feature::lifecycle::write_close(
        &layout,
        &name,
        crate::domain::feature::PromotionOutcome::Abandoned,
    )
    .unwrap();

    let blockers = blocking_descendants(
        &crate::git::System,
        &layout,
        &manifest,
        &feature(&layout, "root"),
    )
    .unwrap();
    let names: Vec<&str> = blockers
        .iter()
        .map(|entry| entry.feature.as_str())
        .collect();
    assert_eq!(
        names,
        ["active"],
        "abandoned history is ignored, but its active child still blocks"
    );
}

#[test]
fn an_integrated_fresh_verified_descendant_does_not_block() {
    let (_guard, layout) = seeded_relation_hall();
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    // child does work and integrates into parent (parent fast-forwarded to
    // child's tip); leaf is a fresh integrated descendant of child.
    let source = commit_on_child(&layout, "child", "work.md", "child work");
    let parent_wt = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("parent").unwrap(),
    );
    let bare = layout.repo_bare(&RepoName::new("api").unwrap());
    crate::git::System
        .add_worktree(&bare, &parent_wt, "parent")
        .unwrap();
    git(&parent_wt, &["merge", "--ff-only", "child"]);
    // The leaf's own branch sits at the recorded source.
    git(&bare, &["branch", "leaf", &source]);

    write_feature(&layout, "parent", None, None);
    write_feature(
        &layout,
        "child",
        Some("parent"),
        Some(passing_receipt(&source, &source)),
    );
    write_feature(
        &layout,
        "leaf",
        Some("child"),
        Some(passing_receipt(&source, &source)),
    );

    let blockers = blocking_descendants(
        &crate::git::System,
        &layout,
        &manifest,
        &feature(&layout, "parent"),
    )
    .unwrap();
    assert!(
        blockers.is_empty(),
        "a fresh integrated descendant must not block: {blockers:?}"
    );
}

#[test]
fn subtree_status_renders_the_tree_in_pre_order_with_depth_and_state() {
    let (_guard, layout) = seeded_relation_hall();
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    write_feature(&layout, "root", None, None);
    write_feature(&layout, "parent", Some("root"), None);
    write_feature(&layout, "leaf", Some("parent"), None);

    let entries = subtree_status(
        &crate::git::System,
        &layout,
        &manifest,
        &FeatureName::new("root").unwrap(),
    )
    .unwrap();

    let rendered: Vec<(String, Option<String>, usize, String)> = entries
        .iter()
        .map(|entry| {
            (
                entry.feature.to_string(),
                entry.parent.as_ref().map(ToString::to_string),
                entry.depth,
                entry.state.to_string(),
            )
        })
        .collect();
    assert_eq!(
        rendered,
        [
            ("root".to_owned(), None, 0, "active".to_owned()),
            (
                "parent".to_owned(),
                Some("root".to_owned()),
                1,
                "active".to_owned()
            ),
            (
                "leaf".to_owned(),
                Some("parent".to_owned()),
                2,
                "active".to_owned()
            ),
        ]
    );
    assert_eq!(entries[0].repos.len(), 1);
}

#[test]
fn subtree_status_populates_plan_gate_run_and_sessions() {
    let (_guard, layout) = seeded_relation_hall();
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let root_name = FeatureName::new("root").unwrap();
    let child_name = FeatureName::new("child").unwrap();
    write_feature(&layout, "root", None, None);
    write_feature(&layout, "child", Some("root"), None);

    // Create an active run receipt with a wave checkpoint for child
    let mut receipt = crate::domain::feature::RunReceipt::start(
        crate::domain::feature::RunId::new("00000000-0000-4000-8000-000000000001").unwrap(),
        child_name.clone(),
        "plans/child/plan.md",
        "hash123",
        crate::domain::feature::RunBaseline::empty(),
        crate::domain::name::SessionId::new("00000000-0000-4000-8000-000000000002").unwrap(),
        crate::domain::provider::Provider::ClaudeCode,
        crate::domain::session::rfc3339_now(),
    );
    receipt
        .checkpoint_wave(
            2,
            "wave 2 done",
            crate::domain::name::SessionId::new("00000000-0000-4000-8000-000000000002").unwrap(),
            crate::domain::provider::Provider::ClaudeCode,
            crate::domain::session::rfc3339_now(),
        )
        .unwrap();
    receipt.write(&layout).unwrap();

    let entries = subtree_status(&crate::git::System, &layout, &manifest, &root_name).unwrap();
    assert_eq!(entries.len(), 2);
    let child_entry = &entries[1];
    assert_eq!(child_entry.feature, child_name);
    assert_eq!(
        child_entry.plan_gate,
        crate::domain::feature::GateState::Pending
    );
    assert_eq!(
        child_entry.run,
        Some(crate::action::feature::relations::TreeRun {
            status: crate::domain::feature::RunStatus::Active,
            last_wave: Some(2),
        })
    );
    assert_eq!(child_entry.sessions, Vec::<String>::new());
}

#[test]
fn subtree_status_reports_non_uuid_session_entries_as_they_are() {
    let (_guard, layout) = seeded_relation_hall();
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let root_name = FeatureName::new("root").unwrap();
    let child_name = FeatureName::new("child").unwrap();
    write_feature(&layout, "root", None, None);
    write_feature(&layout, "child", Some("root"), None);

    // Create a non-UUID session entry under child's sessions dir
    let sess_dir = layout.feature_sessions_dir(&child_name).join("sess-1");
    crate::infra::fs::ensure_dir(&sess_dir).unwrap();

    let entries = subtree_status(&crate::git::System, &layout, &manifest, &root_name).unwrap();
    assert_eq!(entries.len(), 2);
    let child_entry = &entries[1];
    assert_eq!(child_entry.feature, child_name);
    assert_eq!(child_entry.sessions, vec!["sess-1".to_owned()]);
}

// -- receipt freshness ------------------------------------------------------

#[test]
fn a_fresh_receipt_is_fresh_when_source_result_and_checks_match() {
    let (_guard, layout) = seeded_relation_hall();
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let source = commit_on_child(&layout, "child", "work.md", "child work");
    let parent_wt = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("parent").unwrap(),
    );
    let bare = layout.repo_bare(&RepoName::new("api").unwrap());
    crate::git::System
        .add_worktree(&bare, &parent_wt, "parent")
        .unwrap();
    git(&parent_wt, &["merge", "--ff-only", "child"]);

    write_feature(&layout, "parent", None, None);
    write_feature(
        &layout,
        "child",
        Some("parent"),
        Some(passing_receipt(&source, &source)),
    );

    let receipt = feature(&layout, "child").promotions[&RepoName::new("api").unwrap()]
        .integration_receipt
        .clone()
        .unwrap();
    let freshness = receipt_freshness(
        &crate::git::System,
        &layout,
        &manifest,
        &feature(&layout, "child"),
        &feature(&layout, "parent"),
        &RepoName::new("api").unwrap(),
        &receipt,
    )
    .unwrap();
    assert_eq!(freshness, ReceiptFreshness::Fresh);

    // The target is always the immediate parent's branch.
    assert_eq!(receipt.target_branch.as_str(), "parent");
}

#[test]
fn source_movement_and_missing_source_are_stale() {
    let (_guard, layout) = seeded_relation_hall();
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let source = commit_on_child(&layout, "child", "work.md", "first");
    write_feature(&layout, "parent", None, None);
    write_feature(
        &layout,
        "child",
        Some("parent"),
        Some(passing_receipt(&source, &source)),
    );
    let api = RepoName::new("api").unwrap();
    let receipt = feature(&layout, "child").promotions[&api]
        .integration_receipt
        .clone()
        .unwrap();

    // Source moved: another commit lands on the child branch.
    commit_on_child(&layout, "child", "work.md", "second");
    let freshness = receipt_freshness(
        &crate::git::System,
        &layout,
        &manifest,
        &feature(&layout, "child"),
        &feature(&layout, "parent"),
        &api,
        &receipt,
    )
    .unwrap();
    assert!(matches!(freshness, ReceiptFreshness::Stale { .. }));

    // Source deleted: the branch is gone entirely.
    let child_wt = layout.repo_worktree(&api, &BranchName::new("child").unwrap());
    crate::git::System
        .remove_worktree(&layout.repo_bare(&api), &child_wt)
        .unwrap();
    git(&layout.repo_bare(&api), &["branch", "-D", "child"]);
    let freshness = receipt_freshness(
        &crate::git::System,
        &layout,
        &manifest,
        &feature(&layout, "child"),
        &feature(&layout, "parent"),
        &api,
        &receipt,
    )
    .unwrap();
    assert!(
        matches!(freshness, ReceiptFreshness::Stale { .. }),
        "a missing source branch is stale, not integrated: {freshness:?}"
    );
}

#[test]
fn check_drift_and_result_loss_are_stale() {
    let (_guard, layout) = seeded_relation_hall();
    let source = commit_on_child(&layout, "child", "work.md", "first");
    write_feature(&layout, "parent", None, None);
    write_feature(
        &layout,
        "child",
        Some("parent"),
        Some(passing_receipt(&source, &source)),
    );
    let api = RepoName::new("api").unwrap();
    let receipt = feature(&layout, "child").promotions[&api]
        .integration_receipt
        .clone()
        .unwrap();

    // Check drift: the manifest's checks changed since the receipt.
    let origin_url = Manifest::read(&layout).unwrap().unwrap().repos()[0]
        .url()
        .to_owned();
    write_manifest(&layout, &origin_url, &["cargo test"]);
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let freshness = receipt_freshness(
        &crate::git::System,
        &layout,
        &manifest,
        &feature(&layout, "child"),
        &feature(&layout, "parent"),
        &api,
        &receipt,
    )
    .unwrap();
    assert!(matches!(freshness, ReceiptFreshness::Stale { .. }));

    // Result loss: the parent branch no longer contains the result.
    write_manifest(&layout, &origin_url, &["true"]);
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let parent_wt = layout.repo_worktree(&api, &BranchName::new("parent").unwrap());
    if crate::git::System.target_state(&parent_wt).unwrap() != crate::git::TargetState::Repository {
        crate::git::System
            .add_worktree(&layout.repo_bare(&api), &parent_wt, "parent")
            .unwrap();
    }
    git(&parent_wt, &["reset", "--hard", "main"]);
    let freshness = receipt_freshness(
        &crate::git::System,
        &layout,
        &manifest,
        &feature(&layout, "child"),
        &feature(&layout, "parent"),
        &api,
        &receipt,
    )
    .unwrap();
    assert!(matches!(freshness, ReceiptFreshness::Stale { .. }));
}

#[test]
fn failed_evidence_is_failed_not_stale() {
    let (_guard, layout) = seeded_relation_hall();
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let source = commit_on_child(&layout, "child", "work.md", "first");
    let mut receipt = passing_receipt(&source, &source);
    receipt.verification.child = vec![VerificationResult::failed("true", Some(1), "boom")];
    write_feature(&layout, "parent", None, None);
    write_feature(&layout, "child", Some("parent"), Some(receipt.clone()));

    let freshness = receipt_freshness(
        &crate::git::System,
        &layout,
        &manifest,
        &feature(&layout, "child"),
        &feature(&layout, "parent"),
        &RepoName::new("api").unwrap(),
        &receipt,
    )
    .unwrap();
    assert_eq!(freshness, ReceiptFreshness::Failed);

    // And it blocks the parent.
    let blockers = blocking_descendants(
        &crate::git::System,
        &layout,
        &manifest,
        &feature(&layout, "parent"),
    )
    .unwrap();
    assert_eq!(blockers[0].state, FeatureIntegrationState::Failed);
}

#[test]
fn forest_returns_roots_sorted_by_name_with_descendants_in_preorder() {
    let mut features = Vec::new();

    // Roots: "zeta" and "alpha"
    // "zeta" has child "zeta-sub" (depth 1) which has child "zeta-sub-leaf" (depth 2)
    // "alpha" has children "alpha-b" and "alpha-a" (depth 1)
    let mut f_zeta = Feature::new(
        FeatureName::new("zeta").unwrap(),
        BranchName::new("zeta").unwrap(),
    );
    f_zeta.parent = None;
    features.push(f_zeta);

    let mut f_zeta_sub = Feature::new(
        FeatureName::new("zeta-sub").unwrap(),
        BranchName::new("zeta-sub").unwrap(),
    );
    f_zeta_sub.parent = Some(FeatureName::new("zeta").unwrap());
    features.push(f_zeta_sub);

    let mut f_zeta_sub_leaf = Feature::new(
        FeatureName::new("zeta-sub-leaf").unwrap(),
        BranchName::new("zeta-sub-leaf").unwrap(),
    );
    f_zeta_sub_leaf.parent = Some(FeatureName::new("zeta-sub").unwrap());
    features.push(f_zeta_sub_leaf);

    let mut f_alpha = Feature::new(
        FeatureName::new("alpha").unwrap(),
        BranchName::new("alpha").unwrap(),
    );
    f_alpha.parent = None;
    features.push(f_alpha);

    let mut f_alpha_b = Feature::new(
        FeatureName::new("alpha-b").unwrap(),
        BranchName::new("alpha-b").unwrap(),
    );
    f_alpha_b.parent = Some(FeatureName::new("alpha").unwrap());
    features.push(f_alpha_b);

    let mut f_alpha_a = Feature::new(
        FeatureName::new("alpha-a").unwrap(),
        BranchName::new("alpha-a").unwrap(),
    );
    f_alpha_a.parent = Some(FeatureName::new("alpha").unwrap());
    features.push(f_alpha_a);

    let map = build_feature_map(&features);
    let entries = forest(&map);

    let rendered: Vec<(usize, &str)> = entries
        .iter()
        .map(|(depth, f)| (*depth, f.name.as_str()))
        .collect();

    assert_eq!(
        rendered,
        vec![
            (0, "alpha"),
            (1, "alpha-a"),
            (1, "alpha-b"),
            (0, "zeta"),
            (1, "zeta-sub"),
            (2, "zeta-sub-leaf"),
        ]
    );
}

#[test]
fn forest_treats_dangling_parent_feature_as_root_with_descendants_beneath_it() {
    let mut features = Vec::new();

    // "orphan" has a parent that is not in the map ("missing-parent")
    let mut f_orphan = Feature::new(
        FeatureName::new("orphan").unwrap(),
        BranchName::new("orphan").unwrap(),
    );
    f_orphan.parent = Some(FeatureName::new("missing-parent").unwrap());
    features.push(f_orphan);

    // "orphan-child" is a child of "orphan"
    let mut f_orphan_child = Feature::new(
        FeatureName::new("orphan-child").unwrap(),
        BranchName::new("orphan-child").unwrap(),
    );
    f_orphan_child.parent = Some(FeatureName::new("orphan").unwrap());
    features.push(f_orphan_child);

    // "normal" is a regular root
    let mut f_normal = Feature::new(
        FeatureName::new("normal").unwrap(),
        BranchName::new("normal").unwrap(),
    );
    f_normal.parent = None;
    features.push(f_normal);

    let map = build_feature_map(&features);
    let entries = forest(&map);

    let rendered: Vec<(usize, &str)> = entries
        .iter()
        .map(|(depth, f)| (*depth, f.name.as_str()))
        .collect();

    assert_eq!(
        rendered,
        vec![
            (0, "normal"),
            (0, "orphan"),
            (1, "orphan-child"),
        ]
    );
}

#[test]
fn forest_with_empty_map_returns_empty_vec() {
    let features: Vec<Feature> = Vec::new();
    let map = build_feature_map(&features);
    assert!(forest(&map).is_empty());
}
