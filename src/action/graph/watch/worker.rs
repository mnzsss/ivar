//! Background worker thread for automatic graph reindexing.

#![allow(dead_code, clippy::needless_pass_by_value, clippy::collapsible_if)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::action::Failure;
use crate::action::graph::cross_repo;
use crate::action::graph::index;
use crate::action::graph::watch::scopes::{Debouncer, Scope, WatchSet};
use crate::action::progress::Silent;
use crate::git::{Git, System as GitSystem};
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

pub(crate) const TICK: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TargetKind {
    Base,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub scope: Scope,
    pub worktree: Utf8PathBuf,
    pub git_meta: Vec<(Utf8PathBuf, Vec<String>)>,
    pub kind: TargetKind,
}

pub(crate) type Discover = Box<dyn FnMut(&Layout, &GraphDb) -> Vec<Target> + Send>;

/// Constructs base targets from declared repos in the manifest whose worktrees exist.
#[must_use]
pub(crate) fn base_targets(layout: &Layout, manifest: &Manifest) -> Vec<Target> {
    let mut targets = Vec::new();
    for repo in manifest.repos() {
        let worktree = layout.repo_worktree(repo.name(), repo.default_branch());
        if !worktree.as_std_path().exists() {
            continue;
        }

        let dot_git = worktree.join(".git");
        let gitdir = if dot_git.is_file() {
            let Ok(content) = std::fs::read_to_string(dot_git.as_std_path()) else {
                continue;
            };
            let Some(rest) = content.strip_prefix("gitdir: ") else {
                continue;
            };
            let trimmed = rest.trim();
            let p = Utf8Path::new(trimmed);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                worktree.join(p)
            }
        } else if dot_git.is_dir() {
            dot_git
        } else {
            continue;
        };

        let branch = repo.default_branch().as_str();
        let branch_path = Utf8Path::new(branch);
        let branch_parent = branch_path.parent();
        let branch_leaf = branch_path.file_name().unwrap_or(branch).to_owned();

        let branch_ref_dir = match branch_parent {
            Some(p) if !p.as_str().is_empty() => gitdir.join("refs/heads").join(p),
            _ => gitdir.join("refs/heads"),
        };

        let git_meta = vec![
            (
                gitdir.clone(),
                vec!["HEAD".to_owned(), "packed-refs".to_owned()],
            ),
            (branch_ref_dir, vec![branch_leaf]),
        ];

        targets.push(Target {
            scope: Scope::Base {
                repo: repo.name().as_str().to_owned(),
            },
            worktree,
            git_meta,
            kind: TargetKind::Base,
        });
    }
    targets
}

fn reindex(layout: &Layout, db: &GraphDb, target: &Target) -> Result<(), Failure> {
    match target.kind {
        TargetKind::Base => {
            let Scope::Base { ref repo } = target.scope else {
                return Err(Failure::failed(
                    "graph.watch_reindex",
                    "Expected base scope for base target",
                ));
            };
            let _lock = crate::action::graph::lock_index(layout)?;
            let outcome =
                index::index_repo(db, repo, target.worktree.as_std_path(), false, &Silent)
                    .map_err(|err| Failure::failed("graph.watch_reindex", err.to_string()))?;

            if outcome.files_indexed > 0 || outcome.files_deleted > 0 {
                cross_repo::link_cross_repo_edges(db)
                    .map_err(|err| Failure::failed("graph.watch_reindex", err.to_string()))?;
            }
            Ok(())
        }
    }
}

fn handle_fs_event(
    event: &notify::Event,
    watcher: &mut RecommendedWatcher,
    set: &mut WatchSet,
    deb: &mut Debouncer,
    db: &GraphDb,
    targets: &[Target],
    now: Instant,
) {
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }
    if event.need_rescan() {
        for t in targets {
            let _ = db.watch_flag_catchup(&t.scope.key());
            let _ = deb.record(t.scope.clone(), now);
        }
    }
    for path in &event.paths {
        let Ok(utf8_path) = Utf8Path::from_path(path).ok_or(()) else {
            continue;
        };
        if matches!(
            event.kind,
            EventKind::Create(notify::event::CreateKind::Folder)
        ) {
            if let Some(d) = set.new_dir(utf8_path) {
                let _ = watcher.watch(d.as_std_path(), RecursiveMode::NonRecursive);
            }
        }
        if let Some(scope) = set.classify(utf8_path) {
            if deb.record(scope.clone(), now) {
                let _ = db.watch_bump_observed(&scope.key());
            }
        }
    }
}

fn process_due_scopes(
    layout: &Layout,
    db: &GraphDb,
    deb: &mut Debouncer,
    targets: &[Target],
    now: Instant,
) {
    let due_scopes = deb.due(now);
    for scope in due_scopes {
        let Some(target) = targets.iter().find(|t| t.scope == scope) else {
            continue;
        };
        let key = scope.key();
        let (seq, was_catchup) = db
            .watch_scopes()
            .ok()
            .and_then(|rows| {
                rows.into_iter()
                    .find(|r| r.scope == key)
                    .map(|r| (r.observed, r.needs_catchup))
            })
            .unwrap_or((0, false));

        match reindex(layout, db, target) {
            Ok(()) => {
                let _ = db.watch_finish(&key, seq, was_catchup);
            }
            Err(err) => {
                let _ = db.watch_fail(&key, &err.to_string());
            }
        }
    }
}

fn run(
    layout: Layout,
    db_path: Utf8PathBuf,
    mut discover: Discover,
    mut watcher: RecommendedWatcher,
    rx: Receiver<notify::Result<notify::Event>>,
    stop: &AtomicBool,
) {
    let Ok(db) = GraphDb::open(db_path.as_std_path()) else {
        return;
    };

    let mut set = WatchSet::default();
    let mut deb = Debouncer::new(Debouncer::QUIET, Debouncer::CAP);
    let git = GitSystem;

    let targets = discover(&layout, &db);

    for target in &targets {
        let key = target.scope.key();
        if db.watch_register(&key).is_err() {
            continue;
        }

        let tracked = git.tracked_files(&target.worktree).unwrap_or_default();
        let wt_dirs = set.add_worktree(&target.scope, &target.worktree, &tracked);
        for dir in wt_dirs {
            let _ = watcher.watch(dir.as_std_path(), RecursiveMode::NonRecursive);
        }

        for (dir, names) in &target.git_meta {
            let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
            let d = set.add_git_meta(&target.scope, dir, &name_refs);
            let _ = watcher.watch(d.as_std_path(), RecursiveMode::NonRecursive);
        }

        let observed_at_start = db
            .watch_scopes()
            .ok()
            .and_then(|rows| {
                rows.into_iter()
                    .find(|r| r.scope == key)
                    .map(|r| r.observed)
            })
            .unwrap_or(0);

        match reindex(&layout, &db, target) {
            Ok(()) => {
                let _ = db.watch_finish(&key, observed_at_start, true);
            }
            Err(err) => {
                let _ = db.watch_fail(&key, &err.to_string());
            }
        }
    }

    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        let timeout = if let Some(deadline) = deb.next_deadline() {
            let remaining = deadline.saturating_duration_since(now);
            TICK.min(remaining)
        } else {
            TICK
        };

        match rx.recv_timeout(timeout) {
            Ok(Ok(event)) => {
                handle_fs_event(&event, &mut watcher, &mut set, &mut deb, &db, &targets, now);
            }
            Ok(Err(error)) => {
                for t in &targets {
                    let _ = db.watch_flag_catchup(&t.scope.key());
                    let _ = deb.record(t.scope.clone(), now);
                }
                let _ = error;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        let now = Instant::now();
        process_due_scopes(&layout, &db, &mut deb, &targets, now);
    }
}

#[derive(Debug)]
pub(crate) struct Worker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    /// Spawns a background watcher thread with the given discovery callback.
    ///
    /// # Errors
    ///
    /// Returns [`Failure`] if the filesystem watcher cannot be created or the thread cannot be spawned.
    pub(crate) fn spawn(
        layout: Layout,
        db_path: Utf8PathBuf,
        discover: Discover,
    ) -> Result<Self, Failure> {
        let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let watcher = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            notify::Config::default(),
        )
        .map_err(|e| Failure::failed("graph.watch_start", e.to_string()))?;

        let stop = Arc::new(AtomicBool::new(false));
        let handle = std::thread::Builder::new()
            .name("ivar-graph-watch".into())
            .spawn({
                let stop = stop.clone();
                move || run(layout, db_path, discover, watcher, rx, &stop)
            })
            .map_err(|e| Failure::failed("graph.watch_start", e.to_string()))?;

        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }

    #[must_use]
    pub(crate) fn is_running(&self) -> bool {
        !self.stop.load(Ordering::Relaxed) && self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/watch/worker.rs"]
mod tests;
