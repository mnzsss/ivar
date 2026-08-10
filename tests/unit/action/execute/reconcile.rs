#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::execute::prepare::{self as prepare_action, PrepareInput};
use crate::action::feature::create::{
    self as feature_create, CreateInput as FeatureCreateInput,
};
use crate::action::feature::promote::{self, PromoteInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

const GRAPH_JSON: &str = r#"{
    "workstreams": [
        {
            "id": "ws-impl",
            "title": "Implement",
            "operations": ["write-code"],
            "depends_on": [],
            "write_contract": ["src/"]
        }
    ]
}"#;

/// A hall with a seeded repo promoted into the feature, a plan, and a
/// prepared board.
fn hall_with_promoted_worktree() -> (tempfile::TempDir, Utf8PathBuf) {
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

    let origin = seeded_repo(&root.join("origins").join("api"), "main");
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

    feature_create::create(
        &ctx,
        FeatureCreateInput {
            name: "checkout".to_owned(),
            branch: None,
        },
    )
    .unwrap();
    plan_create::create(
        &ctx,
        PlanCreateInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
        },
    )
    .unwrap();

    let graph = root.join("graph.json");
    fs::write_text(&graph, GRAPH_JSON).unwrap();
    prepare_action::prepare(
        &ctx,
        PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: graph.to_string(),
        },
    )
    .unwrap();
    (guard, root)
}

fn persisted(root: &Utf8PathBuf) -> ExecutionBoard {
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    ExecutionBoard::read(&layout, &feature).unwrap().unwrap()
}

#[test]
fn reconcile_appends_the_divergence_without_rewriting_the_plan() {
    let (_guard, root) = hall_with_promoted_worktree();
    let ctx = Ctx::new(root.clone());

    // The executor diverges: README.md gains an uncommitted line in the
    // feature worktree.
    let worktree = root.join(".ivar/repos/api/checkout");
    let readme = worktree.join("README.md");
    let original = fs::read_text(&readme).unwrap().unwrap();
    fs::write_text(&readme, &format!("{original}diverged\n")).unwrap();
    let plan_before = fs::read_text(&root.join("plans/checkout/plan.md"))
        .unwrap()
        .unwrap();

    let report = reconcile(
        &ctx,
        ReconcileInput {
            feature: "checkout".to_owned(),
            workstream: "ws-impl".to_owned(),
            description: "implemented auth differently".to_owned(),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.workstream, "ws-impl");
    assert!(report.value.diff.contains("diverged"));
    assert!(report.value.prior_deviations.is_empty());

    // The journal records it, and the plan.md is byte-for-byte untouched.
    let on_disk = persisted(&root);
    let entry = on_disk.journal.last().unwrap();
    assert_eq!(entry.kind, "reconcile");
    assert_eq!(entry.workstream, "ws-impl");
    assert!(entry.message.contains("implemented auth differently"));
    assert!(entry.message.contains("diverged"));
    assert_eq!(
        fs::read_text(&root.join("plans/checkout/plan.md"))
            .unwrap()
            .unwrap(),
        plan_before,
        "reconcile must never rewrite the plan"
    );
}

#[test]
fn reconcile_records_a_divergence_without_promoted_worktrees() {
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
    feature_create::create(
        &ctx,
        FeatureCreateInput {
            name: "checkout".to_owned(),
            branch: None,
        },
    )
    .unwrap();
    plan_create::create(
        &ctx,
        PlanCreateInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    let graph = root.join("graph.json");
    fs::write_text(&graph, GRAPH_JSON).unwrap();
    prepare_action::prepare(
        &ctx,
        PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: graph.to_string(),
        },
    )
    .unwrap();

    let report = reconcile(
        &ctx,
        ReconcileInput {
            feature: "checkout".to_owned(),
            workstream: "ws-impl".to_owned(),
            description: "no repos promoted".to_owned(),
        },
    )
    .unwrap();

    assert!(report.value.diff.is_empty());
    let on_disk = persisted(&root);
    let entry = on_disk.journal.last().unwrap();
    assert_eq!(entry.kind, "reconcile");
    assert!(entry.message.contains("no uncommitted diff"));
}

#[test]
fn reconcile_is_blocked_without_a_board() {
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
    feature_create::create(
        &ctx,
        FeatureCreateInput {
            name: "checkout".to_owned(),
            branch: None,
        },
    )
    .unwrap();

    let failure = reconcile(
        &ctx,
        ReconcileInput {
            feature: "checkout".to_owned(),
            workstream: "ws-impl".to_owned(),
            description: "nothing".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "execute.board_missing");
}

#[test]
fn reconcile_is_blocked_for_an_unknown_workstream() {
    let (_guard, root) = hall_with_promoted_worktree();
    let ctx = Ctx::new(root.clone());

    let failure = reconcile(
        &ctx,
        ReconcileInput {
            feature: "checkout".to_owned(),
            workstream: "ws-ghost".to_owned(),
            description: "nothing".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "execute.workstream_not_found");
}
