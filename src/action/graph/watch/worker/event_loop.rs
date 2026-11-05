use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::action::graph::watch::scopes::{Debouncer, Scope, WatchSet};
use crate::action::graph::watch::worker::registry::{add_control_dirs, register_and_watch_target};
use crate::action::graph::watch::worker::reindex::reindex;
use crate::action::graph::watch::worker::{Discover, TICK, Target};
use crate::action::memory::drain::drain_one;
use crate::action::memory::reindex;
use crate::action::memory::sweep::sweep_now;
use crate::git::System as GitSystem;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::memory::spool;

const DRAIN_EVERY: Duration = Duration::from_millis(250);
const SWEEP_EVERY: Duration = Duration::from_secs(3600);
const REINDEX_EVERY: Duration = Duration::from_secs(5);

fn reindex_due(last: &mut Option<Instant>, now: Instant) -> bool {
    let due = last.is_none_or(|at| now.duration_since(at) >= REINDEX_EVERY);
    if due {
        *last = Some(now);
    }
    due
}

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
        let classified = set.classify(utf8_path);
        if let Some(scope) = classified {
            deb.clear_failures(&scope);
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
                deb.clear_failures(&scope);
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
                                    deb.clear_failures(&other.scope);
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
                deb.record_failure(scope, now);
            }
        }
    }
}

fn forget_rows_without_target(layout: &Layout, db: &GraphDb, targets: &[Target]) {
    let Ok(_lock) = crate::action::graph::lock_index(layout) else {
        return;
    };
    let keep: BTreeSet<String> = targets.iter().map(|t| t.scope.key()).collect();
    for row in db.watch_scopes().unwrap_or_default() {
        if !keep.contains(&row.scope) {
            let _ = db.watch_forget(&row.scope);
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

    let now = Instant::now();
    for target in &targets {
        register_and_watch_target(
            &layout,
            &db,
            &mut deb,
            target,
            &mut set,
            &mut watcher,
            &git,
            now,
        );
    }
    forget_rows_without_target(&layout, &db, &targets);

    let mut rediscover = false;
    let mut last_drain = Instant::now();
    let mut last_sweep: Option<Instant> = None;
    let mut last_reindex: Option<Instant> = None;

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

            for old_t in &targets {
                if !new_targets.iter().any(|t| t.scope == old_t.scope) {
                    let unwatched = set.remove_scope(&old_t.scope);
                    for p in unwatched {
                        let _ = watcher.unwatch(p.as_std_path());
                    }
                }
            }
            forget_rows_without_target(&layout, &db, &new_targets);

            // Targets new: register, watch, and catch-up
            for new_t in &new_targets {
                if !targets.iter().any(|t| t.scope == new_t.scope) {
                    register_and_watch_target(
                        &layout,
                        &db,
                        &mut deb,
                        new_t,
                        &mut set,
                        &mut watcher,
                        &git,
                        now,
                    );
                }
            }

            targets = new_targets;
        }

        let now = Instant::now();
        process_due_scopes(&layout, &db, &mut deb, &targets, now);

        if reindex_due(&mut last_reindex, Instant::now()) {
            let _ = reindex::refresh_if_free(&layout);
        }

        if last_drain.elapsed() >= DRAIN_EVERY {
            last_drain = Instant::now();
            if spool::has_pending(&layout.memory_spool_dir()) {
                let _ = drain_one(&layout);
            }
        }

        if last_sweep.is_none_or(|at| at.elapsed() >= SWEEP_EVERY) {
            last_sweep = Some(Instant::now());
            let _ = sweep_now(&layout);
        }
    }
}

#[cfg(test)]
#[path = "../../../../../tests/unit/action/graph/watch/event_loop.rs"]
mod tests;
