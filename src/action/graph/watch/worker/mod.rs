//! Background worker thread for automatic graph reindexing.

#![allow(
    clippy::needless_pass_by_value,
    clippy::collapsible_if,
    clippy::too_many_arguments
)]

mod event_loop;
mod registry;
mod reindex;
mod targets;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use camino::Utf8PathBuf;
use notify::{RecommendedWatcher, Watcher};

use crate::action::Failure;
use crate::action::graph::watch::worker::event_loop::run;
pub(crate) use crate::action::graph::watch::worker::targets::{
    Discover, Target, TargetKind, base_commit_for, base_targets, layer_targets,
};
use crate::store::layout::Layout;

pub(crate) const TICK: Duration = Duration::from_millis(50);

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
#[path = "../../../../../tests/unit/action/graph/watch/worker.rs"]
mod tests;
