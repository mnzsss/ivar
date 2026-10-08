#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

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

fn init_repo(root: &Utf8Path, name: &str) -> Utf8PathBuf {
    let repo = root.join(name);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
    std::fs::write(repo.join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init"]);
    repo
}

fn hall_with_repo() -> (TempDir, Layout, Utf8PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    std::fs::create_dir_all(root.join(".ivar")).unwrap();
    let repo = init_repo(&root, "api");
    (tmp, Layout::at(root), repo)
}

fn base_target(name: &str, repo: &Utf8Path) -> Target {
    Target {
        scope: Scope::Base { repo: name.into() },
        worktree: repo.to_owned(),
        git_meta: vec![
            (repo.join(".git"), vec!["HEAD".into(), "packed-refs".into()]),
            (repo.join(".git/refs/heads"), vec!["main".into()]),
        ],
        kind: TargetKind::Base,
    }
}

fn target(repo: &Utf8Path) -> Target {
    base_target("api", repo)
}

fn observed(db: &GraphDb, scope: &str) -> i64 {
    db.watch_scopes()
        .unwrap()
        .into_iter()
        .find(|row| row.scope == scope)
        .unwrap_or_else(|| panic!("no watch scope {scope}"))
        .observed
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

/// One watcher thread handles every event in arrival order, and a reindex
/// finishes its writes before its scope reads as settled. So any event the
/// `api` catch-up caused is handled before the `web` edit made after it,
/// and once `web` has been reindexed, `api`'s count is final.
#[test]
fn an_idle_indexed_repo_never_retriggers_itself() {
    let (_tmp, layout, repo) = hall_with_repo();
    let sentinel = init_repo(layout.root(), "web");
    let targets = vec![target(&repo), base_target("web", &sentinel)];
    let db_path = layout.ivar_dir().join("memory.db");
    let db = GraphDb::open(db_path.as_std_path()).unwrap();
    let _worker = Worker::spawn(
        layout.clone(),
        db_path,
        Box::new(move |_, _| targets.clone()),
    )
    .unwrap();
    wait_for("catch-up", || {
        db.watch_settled(&["base:api", "base:web"]).unwrap()
    });
    let before = observed(&db, "base:api");
    let sentinel_before = observed(&db, "base:web");

    std::fs::write(sentinel.join("src/lib.rs"), "pub fn omega() {}\n").unwrap();
    wait_for("sentinel reindex", || {
        observed(&db, "base:web") > sentinel_before && db.watch_settled(&["base:web"]).unwrap()
    });

    assert_eq!(
        observed(&db, "base:api"),
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

#[test]
fn symlinked_worktree_and_control_dirs_classify_canonical_fs_events() {
    use notify::event::{CreateKind, ModifyKind};
    use notify::{Event, EventKind, Watcher};

    let real_tmp = tempfile::tempdir().unwrap();
    let real_root = Utf8PathBuf::from_path_buf(real_tmp.path().to_path_buf()).unwrap();
    let symlink_tmp = tempfile::tempdir().unwrap();
    let symlink_root = Utf8PathBuf::from_path_buf(symlink_tmp.path().join("symlink_hall")).unwrap();

    crate::infra::fs::create_symlink(&real_root, &symlink_root).unwrap();
    let real_repo = real_root.join(".ivar/repos/api/main");
    std::fs::create_dir_all(real_repo.join("src")).unwrap();
    std::fs::create_dir_all(real_root.join(".ivar/features/feat/sessions")).unwrap();

    git(&real_repo, &["init", "-b", "main"]);
    git(&real_repo, &["config", "user.name", "t"]);
    git(&real_repo, &["config", "user.email", "t@t"]);
    std::fs::write(real_repo.join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    git(&real_repo, &["add", "."]);
    git(&real_repo, &["commit", "-m", "init"]);
    let manifest_content = r#"{
  "version": 4,
  "name": "symlink-hall",
  "providers": {
    "available": ["claude-code"],
    "default": "claude-code"
  },
  "integration": {
    "strategy": "squash",
    "via": "local"
  },
  "repos": [
    {
      "name": "api",
      "url": "https://github.com/example/api",
      "default_branch": "main"
    }
  ]
}"#;
    std::fs::write(real_root.join("ivar.json"), manifest_content).unwrap();

    let sym_layout = Layout::at(symlink_root.clone());
    let sym_manifest = crate::store::manifest::Manifest::read(&sym_layout)
        .unwrap()
        .unwrap();
    let base_targets =
        crate::action::graph::watch::worker::base_targets(&sym_layout, &sym_manifest);
    assert_eq!(base_targets.len(), 1);

    let db_path = real_root.join(".ivar/memory.db");
    let db = GraphDb::open(db_path.as_std_path()).unwrap();

    let mut set = crate::action::graph::watch::scopes::WatchSet::default();
    let (tx, _rx) = std::sync::mpsc::channel();
    let mut watcher = notify::RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        notify::Config::default(),
    )
    .unwrap();

    crate::action::graph::watch::worker::registry::add_control_dirs(
        &sym_layout,
        &mut set,
        &mut watcher,
    );
    let git_sys = crate::git::System;
    let mut deb = crate::action::graph::watch::scopes::Debouncer::new(
        Duration::from_millis(150),
        Duration::from_millis(1000),
    );

    crate::action::graph::watch::worker::registry::register_and_watch_target(
        &sym_layout,
        &db,
        &mut deb,
        &base_targets[0],
        &mut set,
        &mut watcher,
        &git_sys,
        Instant::now(),
    );

    let canonical_file = real_repo.join("src/lib.rs").canonicalize_utf8().unwrap();
    let event = Event {
        kind: EventKind::Modify(ModifyKind::Any),
        paths: vec![canonical_file.as_std_path().to_path_buf()],
        attrs: notify::event::EventAttributes::default(),
    };

    let mut rediscover = false;
    crate::action::graph::watch::worker::event_loop::handle_fs_event(
        &event,
        &mut watcher,
        &mut set,
        &mut deb,
        &db,
        &base_targets,
        Instant::now(),
        &mut rediscover,
    );

    let scope = Scope::Base { repo: "api".into() };
    assert!(
        deb.due(Instant::now() + Duration::from_millis(200))
            .contains(&scope),
        "Canonical file path event must be classified and recorded in debouncer"
    );

    let canonical_session = real_root
        .join(".ivar/features/feat/sessions")
        .canonicalize_utf8()
        .unwrap()
        .join("0a2d7418");
    let control_event = Event {
        kind: EventKind::Create(CreateKind::Folder),
        paths: vec![canonical_session.as_std_path().to_path_buf()],
        attrs: notify::event::EventAttributes::default(),
    };
    crate::action::graph::watch::worker::event_loop::handle_fs_event(
        &control_event,
        &mut watcher,
        &mut set,
        &mut deb,
        &db,
        &base_targets,
        Instant::now(),
        &mut rediscover,
    );
    assert!(
        rediscover,
        "Canonical control dir event must set rediscover"
    );
}
