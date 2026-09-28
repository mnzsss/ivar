//! Watcher module for graph automatic reindexing.

pub mod lease;
pub mod scopes;
pub(crate) mod worker;

use crate::action::graph::watch::lease::Lease;
use crate::action::graph::watch::worker::Worker;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaderState {
    Us,
    Other,
    None,
}

#[derive(Debug)]
pub struct Watch {
    layout: Layout,
    discover: fn() -> worker::Discover,
    leader: Option<(Lease, Worker)>,
}

impl Watch {
    #[must_use]
    pub fn new(layout: Layout) -> Self {
        Self::with_discover(layout, default_discover)
    }

    pub(crate) fn with_discover(layout: Layout, discover: fn() -> worker::Discover) -> Self {
        Self {
            layout,
            discover,
            leader: None,
        }
    }

    pub fn probe(&mut self, db: &GraphDb) -> LeaderState {
        if let Some((_, worker)) = &self.leader {
            if worker.is_running() {
                return LeaderState::Us;
            }
            self.leader = None; // worker died: release the lease so another process can lead
            return LeaderState::None;
        }
        match Lease::try_acquire(&self.layout) {
            Ok(Some(lease)) => {
                if db.watch_mark_all_catchup().is_err() {
                    return LeaderState::None;
                }
                let db_path = self.layout.ivar_dir().join("memory.db");
                match Worker::spawn(self.layout.clone(), db_path, (self.discover)()) {
                    Ok(worker) => {
                        self.leader = Some((lease, worker));
                        LeaderState::Us
                    }
                    Err(_) => LeaderState::None,
                }
            }
            Ok(None) => LeaderState::Other,
            Err(_) => LeaderState::None,
        }
    }
}

fn default_discover() -> worker::Discover {
    Box::new(|layout, _| {
        let mut targets = read_manifest_quiet(layout)
            .map(|m| worker::base_targets(layout, &m))
            .unwrap_or_default();
        targets.extend(worker::layer_targets(layout));
        targets
    })
}

fn read_manifest_quiet(layout: &Layout) -> Option<Manifest> {
    Manifest::read(layout).ok().flatten()
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/watch/probe.rs"]
mod tests;
