//! `ivar automation`: finding the requests a user keeps repeating, and
//! remembering what the human answered to the proposals made from them.

pub mod decision;
pub mod scan;
pub mod sweep;

use rusqlite::Connection;

use crate::domain::automation::{Decision, DecisionKind, clean_artifact};
use crate::error::{Failure, FixAction};
use crate::store::layout::Layout;
use crate::store::memory::db::{MemoryDbError, ReadOnlyOpen, open_current_read_only};

/// Opens `memory.db` read-only for a command that only reads it. `None` when
/// the hall has no database yet: a file with no bytes is what an interrupted
/// first open leaves.
///
/// # Errors
///
/// When `memory.db` was written by another ivar version (older: not yet
/// migrated, newer: unsupported), is not a memory database, or cannot be read.
pub(crate) fn open_read_only(layout: &Layout) -> Result<Option<Connection>, Failure> {
    let path = layout.memory_db();
    if !has_database(layout) {
        return Ok(None);
    }
    match open_current_read_only(path.as_std_path()) {
        Ok(ReadOnlyOpen::Current(conn)) => Ok(Some(conn)),
        Ok(ReadOnlyOpen::Outdated { found, current }) => Err(outdated(found, current)),
        Err(error) => Err(unreadable(&path, &error)),
    }
}

/// Whether `memory.db` exists and has bytes.
pub(crate) fn has_database(layout: &Layout) -> bool {
    std::fs::metadata(layout.memory_db()).is_ok_and(|meta| meta.len() > 0)
}

/// Whether an accepted decision's artifact is still in the hall. Any other
/// decision has none to lose.
pub(crate) fn artifact_exists(layout: &Layout, decision: &Decision) -> bool {
    if decision.kind != DecisionKind::Accepted {
        return true;
    }
    decision
        .artifact
        .as_deref()
        .and_then(|artifact| clean_artifact(artifact).ok())
        .is_some_and(|artifact| layout.root().join(artifact).exists())
}

fn outdated(found: i64, current: i64) -> Failure {
    Failure::blocked(
        "automation.memory_outdated",
        format!(
            "memory.db has schema {found} and this ivar reads schema {current}; \
             this command never migrates it"
        ),
    )
    .fix(
        FixAction::safe(
            "automation.migrate_memory",
            "Migrate memory.db from a session that can write `.ivar/`, then run the command again.",
        )
        .command("ivar memory status"),
    )
}

fn unreadable(path: &camino::Utf8Path, error: &MemoryDbError) -> Failure {
    Failure::failed(
        "automation.memory_unreadable",
        format!("could not read `{path}`: {error}"),
    )
}
