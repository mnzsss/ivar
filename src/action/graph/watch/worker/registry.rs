use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::action::graph::watch::scopes::{Debouncer, WatchSet};
use crate::action::graph::watch::worker::Target;
use crate::action::graph::watch::worker::reindex::reindex;
use crate::domain::name::FeatureName;
use crate::git::{Git, System as GitSystem};
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;

pub(super) fn register_and_watch_target(
    layout: &Layout,
    db: &GraphDb,
    deb: &mut Debouncer,
    target: &Target,
    set: &mut WatchSet,
    watcher: &mut RecommendedWatcher,
    git: &GitSystem,
    now: std::time::Instant,
) {
    let key = target.scope.key();
    if db.watch_register(&key).is_err() {
        return;
    }

    let canonical_worktree = crate::infra::fs::canonicalize(&target.worktree)
        .unwrap_or_else(|_| target.worktree.clone());
    let tracked = git.tracked_files(&canonical_worktree).unwrap_or_default();
    let wt_dirs = set.add_worktree(&target.scope, &canonical_worktree, &tracked);
    for dir in wt_dirs {
        let _ = watcher.watch(dir.as_std_path(), RecursiveMode::NonRecursive);
    }

    for (dir, names) in &target.git_meta {
        let canonical_dir = crate::infra::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone());
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let d = set.add_git_meta(&target.scope, &canonical_dir, &name_refs);
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

    match reindex(layout, db, target) {
        Ok(()) => {
            deb.clear_failures(&target.scope);
            let _ = db.watch_finish(&key, observed_at_start, true);
        }
        Err(err) => {
            let _ = db.watch_fail(&key, &err.to_string());
            deb.record_failure(target.scope.clone(), now);
        }
    }
}

pub(super) fn add_control_dirs(
    layout: &Layout,
    set: &mut WatchSet,
    watcher: &mut RecommendedWatcher,
) {
    let features_dir = crate::infra::fs::canonicalize(&layout.features_dir())
        .unwrap_or_else(|_| layout.features_dir());
    let d = set.add_control(&features_dir);
    let _ = watcher.watch(d.as_std_path(), RecursiveMode::NonRecursive);

    if let Ok(entries) = crate::infra::fs::read_dir(&features_dir) {
        for entry in entries {
            if let Some(name) = entry.file_name()
                && let Ok(feat_name) = FeatureName::new(name)
            {
                let sessions_dir = layout.feature_sessions_dir(&feat_name);
                let canonical_sessions =
                    crate::infra::fs::canonicalize(&sessions_dir).unwrap_or(sessions_dir);
                let sd = set.add_control(&canonical_sessions);
                let _ = watcher.watch(sd.as_std_path(), RecursiveMode::NonRecursive);
            }
        }
    }
}
