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
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

/// A hall with `api` promoted into `checkout`, plus a detached session.
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
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    feature_promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            base: None,
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

/// Undo the read-only guards applied, so TempDir can clean up.
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

fn session_id_of(root: &camino::Utf8Path) -> String {
    let layout = Layout::at(root.to_path_buf());
    let dir = layout
        .feature_dir(&FeatureName::new("checkout").unwrap())
        .join("sessions");
    let entry = fs::read_dir(&dir).unwrap();
    let session_dir = &entry[0];
    session_dir.file_name().unwrap().to_owned()
}

#[test]
fn stop_ends_a_live_session_and_removes_the_view_dir() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let id = session_id_of(&root);
    let session_id = crate::domain::name::SessionId::new(id.clone()).unwrap();
    let view_dir = layout.feature_session(&FeatureName::new("checkout").unwrap(), &session_id);

    assert!(fs::is_dir(&view_dir).unwrap());

    let report = stop(&ctx, StopInput { session: Some(id) }).unwrap();

    assert_eq!(report.value.stopped, 1);
    assert!(
        !fs::is_dir(&view_dir).unwrap(),
        "the view dir must be removed"
    );
    unguard_worktrees(&root);
}

#[test]
fn stop_of_an_already_stopped_session_is_a_no_op() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    let id = session_id_of(&root);

    // First stop: removes the view dir.
    stop(
        &ctx,
        StopInput {
            session: Some(id.clone()),
        },
    )
    .unwrap();

    // Second stop: the view dir is already gone → no-op.
    let report = stop(&ctx, StopInput { session: Some(id) }).unwrap();

    assert_eq!(report.value.stopped, 0, "already-stopped must be a no-op");
    unguard_worktrees(&root);
}

#[test]
fn stop_all_stops_every_live_session() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());

    // Add a second session.
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

    let report = stop(&ctx, StopInput { session: None }).unwrap();

    assert_eq!(report.value.stopped, 2);

    // Both view dirs must be gone.
    let sessions_dir = layout.feature_sessions_dir(&FeatureName::new("checkout").unwrap());
    let entries: Vec<_> = fs::read_dir(&sessions_dir)
        .unwrap()
        .into_iter()
        .filter(|e| e.file_name().is_some_and(|n| !n.starts_with('.')))
        .collect();
    assert!(entries.is_empty(), "all sessions must be stopped");
    unguard_worktrees(&root);
}

#[test]
fn stop_emits_human_output() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    let id = session_id_of(&root);

    let report = stop(&ctx, StopInput { session: Some(id) }).unwrap();

    let mut out = Vec::new();
    report.value.write_human(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Stopped 1 session"), "was: {text}");
    unguard_worktrees(&root);
}
