use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use camino::{Utf8Path, Utf8PathBuf};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::action::graph::watch::scopes::{Debouncer, Scope, WatchSet};
use crate::action::graph::watch::worker::registry::{add_control_dirs, register_and_watch_target};
use crate::action::graph::watch::worker::reindex::reindex;
use crate::action::graph::watch::worker::{Discover, TICK, Target};
use crate::git::System as GitSystem;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;

pub(super) fn handle_fs_event(
    event: &notify::Event,
    watcher: &mut RecommendedWatcher,
    set: &mut WatchSet,
    deb: &mut Debouncer,
    db: &GraphDb,
    targets: &[Target],
    now: Instant,
    rediscover: &mut bool,
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
        if set.is_control(utf8_path) {
            *rediscover = true;
        }
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

pub(super) fn process_due_scopes(
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

        let repo_last_commit_before = if let Scope::Base { ref repo } = target.scope {
            db.get_repo_last_commit(repo).ok().flatten()
        } else {
            None
        };

        match reindex(layout, db, target) {
            Ok(()) => {
                let _ = db.watch_finish(&key, seq, was_catchup);

                // When a base repo's commit changes, invalidate all layer scopes of the same repo.
                if let Scope::Base { ref repo } = target.scope {
                    let repo_last_commit_after = db.get_repo_last_commit(repo).ok().flatten();
                    if repo_last_commit_after != repo_last_commit_before {
                        for other in targets {
                            if let Scope::Layer {
                                repo: ref layer_repo,
                                ..
                            } = other.scope
                            {
                                if layer_repo == repo {
                                    let _ = deb.record(other.scope.clone(), now);
                                    let _ = db.watch_bump_observed(&other.scope.key());
                                }
                            }
                        }
                    }
                }
            }
            Err(err) => {
                let _ = db.watch_fail(&key, &err.to_string());
            }
        }
    }
}

pub(super) fn run(
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

    add_control_dirs(&layout, &mut set, &mut watcher);

    let mut targets = discover(&layout, &db);

    for target in &targets {
        register_and_watch_target(&layout, &db, target, &mut set, &mut watcher, &git);
    }

    let mut rediscover = false;

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
                handle_fs_event(
                    &event,
                    &mut watcher,
                    &mut set,
                    &mut deb,
                    &db,
                    &targets,
                    now,
                    &mut rediscover,
                );
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

        if rediscover {
            rediscover = false;
            add_control_dirs(&layout, &mut set, &mut watcher);
            let new_targets = discover(&layout, &db);

            // Targets gone: remove scope from set, unwatch paths, forget from DB
            for old_t in &targets {
                if !new_targets.iter().any(|t| t.scope == old_t.scope) {
                    let unwatched = set.remove_scope(&old_t.scope);
                    for p in unwatched {
                        let _ = watcher.unwatch(p.as_std_path());
                    }
                    let _ = db.watch_forget(&old_t.scope.key());
                }
            }

            // Targets new: register, watch, and catch-up
            for new_t in &new_targets {
                if !targets.iter().any(|t| t.scope == new_t.scope) {
                    register_and_watch_target(&layout, &db, new_t, &mut set, &mut watcher, &git);
                }
            }

            targets = new_targets;
        }

        let now = Instant::now();
        process_due_scopes(&layout, &db, &mut deb, &targets, now);
    }
}
