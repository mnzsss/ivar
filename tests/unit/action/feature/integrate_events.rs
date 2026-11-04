#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use super::*;
use crate::action::feature::create::CreateInput;
use crate::action::feature::create::create as create_action;
use crate::action::feature::promote::{self, PromoteInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::approve as plan_approve;
use crate::action::plan::create as plan_create;
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{git, hall_root, seeded_repo};

/// A hall with one seeded repo declared, a parent and a child feature, the
/// repo promoted into both, a commit on the child, and the child's plan gate
/// approved. `checks` become the api repo's ordered verification checks.
fn seeded_child_hall(checks: &[&str]) -> (tempfile::TempDir, Utf8PathBuf) {
    seeded_child_hall_with_repos(&["api"], checks)
}

fn seeded_child_hall_with_repos(
    repos: &[&str],
    checks: &[&str],
) -> (tempfile::TempDir, Utf8PathBuf) {
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

    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        repos
            .iter()
            .map(|repo| {
                let origin =
                    seeded_repo(&root.parent().unwrap().join("origins").join(repo), "main");
                Repo::new(
                    RepoName::new(*repo).unwrap(),
                    origin.as_str(),
                    BranchName::new("main").unwrap(),
                )
                .with_checks(checks.iter().map(|c| c.to_string()).collect())
            })
            .collect(),
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();

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
        for repo in repos {
            promote::promote(
                &ctx,
                PromoteInput {
                    feature: feature.to_owned(),
                    repo: (*repo).to_owned(),
                    base: None,
                },
            )
            .unwrap();
        }
    }

    for repo in repos {
        commit_work(&layout, repo, "child", "work\n");
    }

    approve_plan(&ctx, "child");

    (guard, root)
}

fn commit_work(layout: &Layout, repo: &str, branch: &str, content: &str) {
    let worktree = layout.repo_worktree(
        &RepoName::new(repo).unwrap(),
        &BranchName::new(branch).unwrap(),
    );
    std::fs::write(worktree.join("work.md"), content).unwrap();
    git(&worktree, &["add", "work.md"]);
    git(&worktree, &["commit", "-m", "work"]);
}

/// Scaffold the child's SPDD artifacts and approve the plan gate.
fn approve_plan(ctx: &Ctx, feature: &str) {
    plan_create::create(
        ctx,
        plan_create::CreateInput {
            feature: feature.to_owned(),
            artifacts: Vec::new(),
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
        name: None,
    }
}

// -- preflight refusals ------------------------------------------------------

use crate::domain::delivery_event::DeliveryEvent;
use crate::test_support::RecordingSink;
use std::sync::Arc;

#[test]
fn a_local_integration_emits_integrated_and_the_close_it_causes() {
    let (_guard, root) = seeded_child_hall(&["true"]);
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root.clone()).with_delivery_events(sink.clone());

    let report = integrate(&ctx, integrate_input("child")).unwrap();

    let records = sink.records();
    let sources: Vec<_> = records
        .iter()
        .map(|r| r.event.source_event().as_str())
        .collect();
    assert_eq!(sources, vec!["delivery.integrated", "delivery.closed"]);
    match &records[0].event {
        DeliveryEvent::Integrated {
            ok,
            via,
            source_sha,
            merge_commit,
        } => {
            assert!(*ok);
            assert_eq!(via, "local");
            assert_eq!(source_sha, &report.value.repos[0].source_sha);
            assert_eq!(merge_commit, &report.value.repos[0].result_sha);
        }
        other => panic!("expected integrated, got {other:?}"),
    }
    assert_eq!(records[0].scope.feature, "child");
    assert_eq!(records[0].scope.parent_feature, "parent");
    assert!(
        matches!(&records[1].event, DeliveryEvent::Closed { outcome } if outcome == "integrated")
    );
    assert_eq!(
        records[0].attempt_id, records[1].attempt_id,
        "one invocation, one attempt"
    );
}

#[test]
fn a_rerun_that_only_reuses_a_receipt_emits_no_integrated_event() {
    let (_guard, root) = seeded_child_hall(&["true"]);
    integrate(&Ctx::new(root.clone()), integrate_input("child")).unwrap();
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());

    integrate(&ctx, integrate_input("child")).unwrap();

    assert!(
        sink.records()
            .iter()
            .all(|r| r.event.source_event().as_str() != "delivery.integrated")
    );
}

#[test]
fn a_failed_integration_check_emits_integrated_with_ok_false() {
    let (_guard, root) = seeded_child_hall(&["true", "exit 1"]);
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());

    integrate(&ctx, integrate_input("child")).unwrap();

    let records = sink.records();
    let integrated = records
        .iter()
        .find(|r| r.event.source_event().as_str() == "delivery.integrated")
        .expect("an integrated event");
    assert!(matches!(
        &integrated.event,
        DeliveryEvent::Integrated { ok: false, .. }
    ));
    assert_eq!(integrated.event.importance().get(), 7);
}

#[test]
fn an_integrated_repo_keeps_its_event_when_a_later_repo_errors() {
    let (_guard, root) = seeded_child_hall_with_repos(&["api", "web"], &["true"]);
    commit_work(&Layout::at(root.clone()), "web", "parent", "conflicting\n");
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());

    let failure = integrate(&ctx, integrate_input("child")).unwrap_err();

    assert_eq!(failure.code, "integration.repo_failed");
    let records = sink.records();
    let integrated: Vec<_> = records
        .iter()
        .filter(|r| r.event.source_event().as_str() == "delivery.integrated")
        .collect();
    assert_eq!(integrated.len(), 1);
    assert_eq!(integrated[0].scope.repo, "api");
    assert!(matches!(
        &integrated[0].event,
        DeliveryEvent::Integrated { ok: true, .. }
    ));
}
