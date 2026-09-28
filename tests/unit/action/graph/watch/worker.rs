#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use tempfile::TempDir;

use crate::action::graph::watch::scopes::Scope;
use crate::action::graph::watch::worker::{Target, TargetKind, Worker};
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;

fn git(dir: &Utf8Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

fn hall_with_repo() -> (TempDir, Layout, Utf8PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    std::fs::create_dir_all(root.join(".ivar")).unwrap();
    let repo = root.join("api");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
    std::fs::write(repo.join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init"]);
    (tmp, Layout::at(root), repo)
}

fn target(repo: &Utf8Path) -> Target {
    Target {
        scope: Scope::Base { repo: "api".into() },
        worktree: repo.to_owned(),
        git_meta: vec![
            (repo.join(".git"), vec!["HEAD".into(), "packed-refs".into()]),
            (repo.join(".git/refs/heads"), vec!["main".into()]),
        ],
        kind: TargetKind::Base,
    }
}

fn names(db: &GraphDb) -> Vec<String> {
    db.conn()
        .prepare("SELECT name FROM symbols WHERE repo = 'api' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn start(layout: &Layout, repo: &Utf8Path) -> (Worker, GraphDb) {
    let db_path = layout.ivar_dir().join("memory.db");
    let db = GraphDb::open(db_path.as_std_path()).unwrap();
    let t = target(repo);
    let worker = Worker::spawn(
        layout.clone(),
        db_path,
        Box::new(move |_, _| vec![t.clone()]),
    )
    .unwrap();
    (worker, db)
}

#[test]
fn the_worker_catches_up_then_reindexes_a_commit_without_any_command() {
    let (_tmp, layout, repo) = hall_with_repo();
    let (worker, db) = start(&layout, &repo);

    wait_for("catch-up", || db.watch_settled(&["base:api"]).unwrap());
    assert_eq!(names(&db), vec!["alpha"]);

    std::fs::write(repo.join("src/lib.rs"), "pub fn beta() {}\n").unwrap();
    git(&repo, &["commit", "-am", "rename"]);

    wait_for("reindex", || {
        names(&db) == vec!["beta"] && db.watch_settled(&["base:api"]).unwrap()
    });
    let head = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(
        db.get_repo_last_commit("api").unwrap().as_deref(),
        Some(head.trim())
    );
    assert!(worker.is_running());
}

#[test]
fn an_uncommitted_new_file_is_indexed() {
    let (_tmp, layout, repo) = hall_with_repo();
    let (_worker, db) = start(&layout, &repo);
    wait_for("catch-up", || db.watch_settled(&["base:api"]).unwrap());

    std::fs::create_dir_all(repo.join("src/fresh")).unwrap();
    std::fs::write(repo.join("src/fresh/mod.rs"), "pub fn gamma() {}\n").unwrap();

    wait_for("new dir file", || names(&db).contains(&"gamma".to_owned()));
}

#[test]
fn an_idle_indexed_repo_never_retriggers_itself() {
    let (_tmp, layout, repo) = hall_with_repo();
    let (_worker, db) = start(&layout, &repo);
    wait_for("catch-up", || db.watch_settled(&["base:api"]).unwrap());
    let before = db.watch_scopes().unwrap()[0].observed;

    std::thread::sleep(Duration::from_millis(1500));

    assert_eq!(
        db.watch_scopes().unwrap()[0].observed,
        before,
        "git status / DB writes must not loop"
    );
}

#[test]
fn dropping_the_worker_stops_it_within_a_tick() {
    let (_tmp, layout, repo) = hall_with_repo();
    let (worker, _db) = start(&layout, &repo);
    let started = Instant::now();
    drop(worker);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_layer_target_is_reindexed_on_an_uncommitted_edit_against_the_indexed_base() {
    let (_tmp, layout, repo) = hall_with_repo();
    let db_path = layout.ivar_dir().join("memory.db");
    let db = GraphDb::open(db_path.as_std_path()).unwrap();
    crate::action::graph::index::index_repo(
        &db,
        "api",
        repo.as_std_path(),
        false,
        &crate::infra::progress::Silent,
    )
    .unwrap();
    let layer = Target {
        scope: Scope::Layer {
            feature: "feat".into(),
            repo: "api".into(),
        },
        worktree: repo.clone(),
        git_meta: vec![(repo.join(".git"), vec!["HEAD".into()])],
        kind: TargetKind::Layer {
            promotion_base: None,
        },
    };
    let _worker = Worker::spawn(
        layout.clone(),
        db_path,
        Box::new(move |_, _| vec![layer.clone()]),
    )
    .unwrap();
    wait_for("layer catch-up", || {
        db.watch_settled(&["layer:feat:api"]).unwrap()
    });

    std::fs::write(repo.join("src/extra.rs"), "pub fn delta() {}\n").unwrap();

    wait_for("layer reindex", || {
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM symbols WHERE repo LIKE 'api/%' AND name = 'delta'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 1
            && db.watch_settled(&["layer:feat:api"]).unwrap()
    });
}
