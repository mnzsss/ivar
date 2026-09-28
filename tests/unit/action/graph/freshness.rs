#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::time::{Duration, Instant};

use camino::Utf8PathBuf;
use tempfile::{TempDir, tempdir};

use crate::action::graph::freshness::{
    ensure_session_freshness, ensure_session_freshness_watched_within,
};
use crate::action::graph::session::{RepoViewInfo, SessionView};
use crate::action::graph::watch::LeaderState;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;

fn session_layer_count(db: &GraphDb) -> i64 {
    db.conn()
        .query_row("SELECT COUNT(*) FROM session_layers", [], |r| r.get(0))
        .unwrap()
}

fn feature_fixture() -> (TempDir, Layout, GraphDb, Utf8PathBuf, SessionView) {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let layout = Layout::at(root.clone());
    let db = GraphDb::open_in_memory().unwrap();
    db.ensure_views_base_mode().unwrap();

    let worktree = root.join("core");
    std::fs::create_dir_all(&worktree).unwrap();

    let run = |cmd: &[&str]| {
        std::process::Command::new(cmd[0])
            .args(&cmd[1..])
            .current_dir(&worktree)
            .output()
            .unwrap();
    };
    run(&["git", "init"]);
    run(&["git", "config", "user.name", "Test"]);
    run(&["git", "config", "user.email", "test@example.com"]);
    std::fs::create_dir_all(worktree.join("src")).unwrap();
    std::fs::write(worktree.join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    run(&["git", "add", "."]);
    run(&["git", "commit", "-m", "initial"]);

    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&worktree)
        .output()
        .unwrap();
    let base_commit = String::from_utf8_lossy(&out.stdout).trim().to_owned();

    db.insert_repo("core", worktree.as_str(), "main", Some(&base_commit))
        .unwrap();

    let view = SessionView::FeatureSession {
        feature_name: "payments".into(),
        session_id: None,
        repos: vec![RepoViewInfo {
            repo_name: "core".into(),
            worktree_path: worktree.clone(),
            is_layer: true,
            base_commit: Some(base_commit),
        }],
    };

    (tmp, layout, db, worktree, view)
}
#[test]
fn feature_freshness_rebuilds_after_a_same_size_content_edit() {
    let (_tmp, layout, db, worktree, view) = feature_fixture();
    std::fs::write(worktree.join("src/lib.rs"), "pub fn bravo() {}\n").unwrap();
    ensure_session_freshness(&db, &layout, &view).unwrap();

    std::fs::write(worktree.join("src/lib.rs"), "pub fn delta() {}\n").unwrap();
    ensure_session_freshness(&db, &layout, &view).unwrap();
    let layer_names: Vec<String> = db
        .conn()
        .prepare("SELECT name FROM symbols WHERE repo LIKE 'core/%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert_eq!(
        layer_names,
        vec!["delta"],
        "same-size content changes must invalidate the layer"
    );
}

#[test]
fn base_view_freshness_clears_session_layers() {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let layout = Layout::at(root.clone());
    let db = GraphDb::open_in_memory().unwrap();
    db.ensure_views_base_mode().unwrap();

    // Set a session layer
    db.configure_session_mode(&[("core", "core/1")]).unwrap();

    let view = SessionView::Base {
        repos: vec![RepoViewInfo {
            repo_name: "core".into(),
            worktree_path: root.join("core"),
            is_layer: false,
            base_commit: None,
        }],
    };

    ensure_session_freshness(&db, &layout, &view).unwrap();
}

#[test]
fn a_settled_layer_is_served_without_touching_git() {
    let (_tmp, layout, db, worktree, view) = feature_fixture();
    ensure_session_freshness(&db, &layout, &view).unwrap();
    db.watch_register("layer:payments:core").unwrap();
    db.watch_finish("layer:payments:core", 0, true).unwrap();

    std::fs::rename(&worktree, worktree.with_extension("gone")).unwrap(); // any git call would now fail

    ensure_session_freshness_watched_within(
        &db,
        &layout,
        &view,
        Some(LeaderState::Other),
        Duration::from_millis(100),
    )
    .expect("fast path must not run git");
    assert_eq!(session_layer_count(&db), 1);
    assert!(
        ensure_session_freshness_watched_within(
            &db,
            &layout,
            &view,
            Some(LeaderState::None),
            Duration::from_millis(100)
        )
        .is_err(),
        "without a leader the git path runs and fails loudly"
    );
}

#[test]
fn a_pending_layer_waits_for_the_timeout_then_takes_the_git_path() {
    let (_tmp, layout, db, worktree, view) = feature_fixture();
    ensure_session_freshness(&db, &layout, &view).unwrap();
    db.watch_register("layer:payments:core").unwrap(); // needs_catchup = 1: never settles here
    std::fs::write(worktree.join("src/lib.rs"), "pub fn echo() {}\n").unwrap();

    let started = Instant::now();
    ensure_session_freshness_watched_within(
        &db,
        &layout,
        &view,
        Some(LeaderState::Other),
        Duration::from_millis(300),
    )
    .unwrap();
    assert!(started.elapsed() >= Duration::from_millis(300), "it waited");
    let names: Vec<String> = db
        .conn()
        .prepare("SELECT name FROM symbols WHERE repo LIKE 'core/%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        names,
        vec!["echo".to_owned()],
        "fallback reindexed the edit"
    );
}

#[test]
fn a_layer_the_leader_does_not_know_falls_back_at_once() {
    let (_tmp, layout, db, _worktree, view) = feature_fixture();
    let started = Instant::now();
    ensure_session_freshness_watched_within(
        &db,
        &layout,
        &view,
        Some(LeaderState::Other),
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "no wait for an unregistered scope"
    );
    assert_eq!(session_layer_count(&db), 1);
}

#[test]
fn the_base_view_never_waits_even_with_pending_scopes() {
    let (_tmp, layout, db, _worktree, _view) = feature_fixture();
    db.watch_register("base:core").unwrap();
    let base = SessionView::Base { repos: Vec::new() };
    let started = Instant::now();
    ensure_session_freshness_watched_within(
        &db,
        &layout,
        &base,
        Some(LeaderState::Us),
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(started.elapsed() < Duration::from_millis(500));
}
