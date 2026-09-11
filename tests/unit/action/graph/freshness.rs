#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;
use tempfile::tempdir;

use crate::action::graph::freshness::ensure_session_freshness;
use crate::action::graph::session::{RepoViewInfo, SessionView};
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;

#[test]
fn feature_freshness_rebuilds_after_a_same_size_content_edit() {
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
    let base_commit = String::from_utf8_lossy(&out.stdout).trim().to_string();

    db.insert_repo("core", worktree.as_str(), "main", Some(&base_commit)).unwrap();

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

    std::fs::write(worktree.join("src/lib.rs"), "pub fn bravo() {}\n").unwrap();
    ensure_session_freshness(&db, &layout, &view).unwrap();
    let first = db.get_layer_record("payments", "core").unwrap().unwrap().fingerprint.unwrap();

    std::fs::write(worktree.join("src/lib.rs"), "pub fn delta() {}\n").unwrap();
    ensure_session_freshness(&db, &layout, &view).unwrap();
    let second = db.get_layer_record("payments", "core").unwrap().unwrap().fingerprint.unwrap();

    assert_ne!(first, second, "same-size content changes must invalidate the layer");
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
