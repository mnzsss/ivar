#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput};
use crate::action::feature::promote::{self as feature_promote, PromoteInput};
use crate::action::hall::{self, InitInput};
use crate::action::session::start::{self as session_start, StartInput};
use crate::domain::name::{BranchName, FeatureName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

fn hall_with_two_sessions() -> (tempfile::TempDir, Utf8PathBuf) {
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
    let api_origin = seeded_repo(&origins.join("api"), "main");
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            api_origin.as_str(),
            BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    feature_create::create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    feature_promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
        },
    )
    .unwrap();

    // Two detached sessions on the same feature.
    session_start::start(
        &ctx,
        StartInput {
            feature: Some("checkout".to_owned()),
            resume: false,
            provider: None,
            detached: true,
            relay: false,
        },
    )
    .unwrap();
    session_start::start(
        &ctx,
        StartInput {
            feature: Some("checkout".to_owned()),
            resume: false,
            provider: None,
            detached: true,
            relay: false,
        },
    )
    .unwrap();

    (guard, root)
}

fn unguard_worktrees(root: &camino::Utf8Path) {
    let repos = root.join(".ivar/repos");
    if !fs::is_dir(&repos).unwrap() {
        return;
    }
    for repo in fs::read_dir(&repos).unwrap() {
        for worktree in fs::read_dir(&repo).unwrap() {
            let _ = fs::restore_write_bits(&worktree);
        }
    }
}

#[test]
fn prune_does_not_touch_live_sessions() {
    let (_guard, root) = hall_with_two_sessions();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let sessions_dir = layout.feature_sessions_dir(&FeatureName::new("checkout").unwrap());

    let before_count = count_entries(&sessions_dir, |n| !n.starts_with('.'));

    let report = prune(&ctx).unwrap();

    assert_eq!(report.value.pruned, 0, "live sessions must not be pruned");
    let after_count = count_entries(&sessions_dir, |n| !n.starts_with('.'));
    assert_eq!(before_count, after_count, "session dirs must remain");
    unguard_worktrees(&root);
}

/// A stale orphan: a view dir under the feature's session tree with no
/// `state.json` — what a session from before session records looked like.
fn orphan_view_dir(layout: &Layout) -> Utf8PathBuf {
    let session_id =
        crate::domain::name::SessionId::new("9a8b7c6d-5e4f-4a3b-9c2d-1e0f1a2b3c4d".to_owned())
            .unwrap();
    let view_dir = layout.feature_session(&FeatureName::new("checkout").unwrap(), &session_id);
    fs::ensure_dir(&view_dir).unwrap();
    view_dir
}

#[test]
fn prune_removes_dead_sessions_and_their_view_dirs() {
    let (_guard, root) = hall_with_two_sessions();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let sessions_dir = layout.feature_sessions_dir(&FeatureName::new("checkout").unwrap());

    let before_count = count_entries(&sessions_dir, |n| !n.starts_with('.'));
    let orphan = orphan_view_dir(&layout);
    assert!(fs::is_dir(&orphan).unwrap());

    let report = prune(&ctx).unwrap();

    assert_eq!(report.value.pruned, 1, "the dead orphan must be pruned");
    assert!(
        !fs::is_dir(&orphan).unwrap(),
        "the orphan view dir must be gone"
    );
    let after_count = count_entries(&sessions_dir, |n| !n.starts_with('.'));
    assert_eq!(
        after_count, before_count,
        "the live sessions must remain and the orphan must be gone"
    );
    unguard_worktrees(&root);
}

#[test]
fn prune_refuses_a_view_dir_with_pending_writes() {
    let (_guard, root) = hall_with_two_sessions();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());

    // A dead view dir with a pending conversion: the `.converting` marker
    // exists under the feature, and the orphan's record was never written.
    let orphan = orphan_view_dir(&layout);
    let feature_dir = layout.feature_dir(&FeatureName::new("checkout").unwrap());
    fs::ensure_dir(&feature_dir).unwrap();
    let converting_path = feature_dir.join(".converting");
    fs::write_text(&converting_path, "{}").unwrap();

    let failure = prune(&ctx).unwrap_err();

    assert_eq!(failure.code, "session.prune_locked");
    assert!(
        failure.what.contains(".converting"),
        "the refusal must name the lock: {}",
        failure.what
    );
    assert!(
        fs::is_dir(&orphan).unwrap(),
        "a refused prune must not remove anything"
    );

    // Cleanup
    let _ = std::fs::remove_file(&converting_path);
    unguard_worktrees(&root);
}

#[test]
fn prune_emits_human_output() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());

    let report = prune(&ctx).unwrap();

    let mut out = Vec::new();
    report.value.write_human(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Pruned 0 dead"), "was: {text}");
    unguard_worktrees(&root);
}

/// Count directory entries matching a filter.
fn count_entries<F>(dir: &camino::Utf8Path, filter: F) -> usize
where
    F: Fn(&str) -> bool,
{
    fs::read_dir(dir)
        .unwrap()
        .into_iter()
        .filter(|e| e.file_name().is_some_and(&filter))
        .count()
}

/// A hall with one detached session (no second session).
fn hall_with_detached_session() -> (tempfile::TempDir, Utf8PathBuf) {
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
    let api_origin = seeded_repo(&origins.join("api"), "main");
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            api_origin.as_str(),
            BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    feature_create::create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    feature_promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
        },
    )
    .unwrap();

    session_start::start(
        &ctx,
        StartInput {
            feature: Some("checkout".to_owned()),
            resume: false,
            provider: None,
            detached: true,
            relay: false,
        },
    )
    .unwrap();

    (guard, root)
}
