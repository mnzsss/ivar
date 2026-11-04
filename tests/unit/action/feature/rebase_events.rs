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
use crate::domain::name::{BranchName, HallName};
use crate::domain::provider::Provider;
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

fn head_of(worktree: &Utf8PathBuf) -> String {
    let out = std::process::Command::new("git")
        .args(["-C", worktree.as_str(), "rev-parse", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
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

use crate::domain::delivery_event::DeliveryEvent;
use crate::test_support::RecordingSink;
use std::sync::Arc;

#[test]
fn a_rebased_repo_emits_one_rebased_event_with_the_heads_before_and_after() {
    let (_guard, root) = hall_with_promoted_feature();
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root.clone()).with_delivery_events(sink.clone());
    let feature_wt = root.join(".ivar/repos/api/checkout");
    std::fs::write(feature_wt.join("feat.txt"), "feature\n").unwrap();
    git(&feature_wt, &["add", "feat.txt"]);
    git(&feature_wt, &["commit", "-m", "feature work"]);
    advance_main(&root);
    let before = head_of(&feature_wt);

    rebase(&ctx, offline_input("checkout")).unwrap();

    let after = head_of(&feature_wt);
    let records = sink.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].scope.repo, "api");
    match &records[0].event {
        DeliveryEvent::Rebased {
            conflicted,
            sha_before,
            sha_after,
            ..
        } => {
            assert!(!conflicted);
            assert_eq!(sha_before.as_deref(), Some(before.as_str()));
            assert_eq!(sha_after.as_deref(), Some(after.as_str()));
        }
        other => panic!("expected rebased, got {other:?}"),
    }
}

#[test]
fn a_conflicting_rebase_emits_rebased_with_conflicted_true_and_failure_importance() {
    let (_guard, root) = hall_with_promoted_feature();
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root.clone()).with_delivery_events(sink.clone());
    let feature_wt = root.join(".ivar/repos/api/checkout");
    let main_wt = root.join(".ivar/repos/api/main");
    std::fs::write(feature_wt.join("README.md"), "feature\n").unwrap();
    git(&feature_wt, &["add", "README.md"]);
    git(&feature_wt, &["commit", "-m", "feature edit"]);
    std::fs::write(main_wt.join("README.md"), "main\n").unwrap();
    git(&main_wt, &["add", "README.md"]);
    git(&main_wt, &["commit", "-m", "main edit"]);

    rebase(&ctx, offline_input("checkout")).unwrap();

    let records = sink.records();
    assert_eq!(records.len(), 1);
    assert!(matches!(
        &records[0].event,
        DeliveryEvent::Rebased {
            conflicted: true,
            ..
        }
    ));
    assert_eq!(records[0].event.importance().get(), 7);
}

#[test]
fn a_skipped_dirty_worktree_emits_nothing() {
    let (_guard, root) = hall_with_promoted_feature();
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root.clone()).with_delivery_events(sink.clone());
    std::fs::write(root.join(".ivar/repos/api/checkout/dirty.txt"), "wip\n").unwrap();

    rebase(&ctx, offline_input("checkout")).unwrap();

    assert!(sink.records().is_empty());
}
