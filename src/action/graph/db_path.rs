//! The graph database's path and its index lock.

use camino::Utf8PathBuf;

use crate::error::{Failure, FixAction};
use crate::store::layout::Layout;
use crate::store::memory::split::{SplitError, ensure_split};

pub(crate) fn lock_index(layout: &Layout) -> Result<std::fs::File, Failure> {
    let lock_path = layout.graph_lock();
    let lock_file = std::fs::File::create(lock_path.as_std_path()).map_err(|err| {
        Failure::failed(
            "graph.lock_failed",
            format!("Failed to create/open index lock file at {lock_path}: {err}"),
        )
    })?;
    lock_file.lock().map_err(|err| {
        Failure::failed(
            "graph.lock_failed",
            format!("Failed to acquire exclusive lock on {lock_path}: {err}"),
        )
    })?;
    Ok(lock_file)
}

/// The graph database's path, after moving a pre-split `.ivar/memory.db`
/// to `.ivar/graph.db`. Every command that opens the graph comes through
/// here; best-effort hook recorders use [`Layout::graph_db`] and never move.
///
/// Never call this while holding [`lock_index`]: the split takes the same
/// `graph.lock`, and a second `flock` in one process waits forever.
///
/// # Errors
///
/// `memory.split_busy` while another process holds a pre-split database
/// open; `memory.split_failed` when the split itself fails.
pub fn graph_db_path(layout: &Layout) -> Result<Utf8PathBuf, Failure> {
    ensure_split(layout).map_err(|error| split_failure(layout, error))?;
    Ok(layout.graph_db())
}

/// The failure a command reports when the database split did not finish.
pub(crate) fn split_failure(layout: &Layout, error: SplitError) -> Failure {
    match error {
        SplitError::Busy => Failure::blocked(
            "memory.split_busy",
            format!(
                "`{}` is still the pre-split graph database and another process holds it open",
                layout.memory_db()
            ),
        )
        .expected("no process from an older ivar holding the hall's databases")
        .actual("its journal mode could not leave WAL")
        .fix(FixAction::safe(
            "memory.stop_old_ivar",
            "Stop the sessions and MCP servers started by an older ivar \
             (`ivar session stop`, or restart the harness), then run the command again.",
        )),
        other => Failure::failed(
            "memory.split_failed",
            format!("could not split the hall databases: {other}"),
        ),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/db_path.rs"]
mod db_path_tests;
