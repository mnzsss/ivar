//! The one-time move of a pre-split `.ivar/memory.db` (the code graph) to
//! `.ivar/graph.db`, followed by the creation of the hall memory database.

use std::path::Path;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use rusqlite::{Connection, OpenFlags};

use super::db::{MemoryDb, MemoryDbError};
use super::schema::APPLICATION_ID;
use crate::store::layout::Layout;

/// Why the split could not finish.
#[derive(Debug, thiserror::Error)]
pub enum SplitError {
    /// Another connection holds the legacy database; nothing was moved.
    #[error("another process still has the legacy graph database `.ivar/memory.db` open")]
    Busy,
    /// An orphan `memory.db-wal` with frames would replay into a new database.
    #[error(
        "`.ivar/memory.db-wal` holds frames but `.ivar/memory.db` is missing; move the WAL aside to recreate the memory database"
    )]
    StaleWal,
    /// The staged `memory.db.new` kept WAL frames after its last connection closed.
    #[error("`.ivar/memory.db.new` could not fold its WAL into the file; nothing was renamed")]
    StagedWal,
    /// A filesystem call failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// An `SQLite` call on the legacy database failed.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    /// The new memory database could not be created.
    #[error(transparent)]
    Memory(#[from] MemoryDbError),
}

/// Whether `path` is a code graph database written before the split.
///
/// # Errors
///
/// Any `SQLite` error opening or reading the file, for example when it is not
/// a database at all or another connection holds it locked.
pub fn is_legacy_graph_db(path: &Path) -> rusqlite::Result<bool> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(Duration::from_millis(250))?;
    let application_id: i32 = conn.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    if application_id == APPLICATION_ID {
        return Ok(false);
    }
    // `graph_misses` only appeared in v0.11.0, so a graph last written by
    // v0.10.0 carries just these two.
    let graph_tables: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
          WHERE type = 'table' AND name IN ('files', 'symbols')",
        [],
        |row| row.get(0),
    )?;
    Ok(graph_tables == 2)
}

/// [`is_legacy_graph_db`], with a lock held by another connection reported
/// as [`SplitError::Busy`]. An unreadable file is not legacy: an unreadable
/// memory.db is the memory database's problem (the doctor reports it), never
/// a reason to stop the graph from opening.
///
/// # Errors
///
/// [`SplitError::Busy`] while the file is locked, [`SplitError::Sqlite`] for
/// any other failure that does not mean the file is unreadable.
pub fn probe_legacy_graph_db(path: &Path) -> Result<bool, SplitError> {
    use rusqlite::ErrorCode::{DatabaseBusy, DatabaseCorrupt, DatabaseLocked, NotADatabase};
    match is_legacy_graph_db(path) {
        Ok(legacy) => Ok(legacy),
        Err(error) => match error.sqlite_error_code() {
            Some(DatabaseBusy | DatabaseLocked) => Err(SplitError::Busy),
            Some(NotADatabase | DatabaseCorrupt) => Ok(false),
            _ => Err(error.into()),
        },
    }
}

/// Moves a legacy `.ivar/memory.db` to `.ivar/graph.db` and creates the hall
/// memory database when it is missing. Idempotent and resumable after a crash.
///
/// # Errors
///
/// [`SplitError::Busy`] while another connection holds the legacy database,
/// [`SplitError::StaleWal`] when an orphan WAL would meet the new database,
/// and the underlying error when a lock, rename or open fails.
pub fn ensure_split(layout: &Layout) -> Result<(), SplitError> {
    let graph = layout.graph_db();
    let memory = layout.memory_db();
    if graph.exists() && memory.exists() {
        return Ok(());
    }
    let _locks = lock_both(layout)?;
    move_if_legacy(&memory, &graph)?;
    if !memory.exists() {
        create_memory_db(&memory)?;
    }
    Ok(())
}

/// Moves a legacy `.ivar/memory.db` to `.ivar/graph.db` without creating the
/// memory database, for callers about to delete it.
///
/// # Errors
///
/// As [`ensure_split`], except that nothing is ever created.
pub fn finish_legacy_move(layout: &Layout) -> Result<(), SplitError> {
    let graph = layout.graph_db();
    let memory = layout.memory_db();
    if graph.exists() || !memory.exists() {
        return Ok(());
    }
    let _locks = lock_both(layout)?;
    move_if_legacy(&memory, &graph)
}

/// Always memory.lock before graph.lock. memory.lock is also the index lock
/// of binaries older than the split, so this excludes their indexers too.
fn lock_both(layout: &Layout) -> Result<[std::fs::File; 2], SplitError> {
    let memory_lock = lock(&layout.memory_lock())?;
    Ok([memory_lock, lock(&layout.graph_lock())?])
}

fn move_if_legacy(memory: &Utf8Path, graph: &Utf8Path) -> Result<(), SplitError> {
    if !graph.exists() && memory.exists() && probe_legacy_graph_db(memory.as_std_path())? {
        move_legacy(memory, graph)?;
    }
    Ok(())
}

/// `MemoryDb::open` creates the file before its migration commits, so the
/// database is built at a sibling path and renamed into place: a crash
/// leaves only the sibling, which the next call discards. The staged file is
/// in WAL mode, so its WAL is folded in and the connection closed before the
/// rename; renaming only the main file would leave the schema behind.
fn create_memory_db(memory: &Utf8Path) -> Result<(), SplitError> {
    let staged = Utf8PathBuf::from(format!("{memory}.new"));
    for suffix in ["", "-journal", "-wal", "-shm"] {
        remove_if_present(&format!("{staged}{suffix}"))?;
    }
    clear_orphan_sidecars(memory)?;
    let db = MemoryDb::open(staged.as_std_path())?;
    let busy: i64 = db
        .conn()
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .map_err(MemoryDbError::from)?;
    if busy != 0 {
        return Err(SplitError::StagedWal);
    }
    db.close()?;
    let staged_wal = format!("{staged}-wal");
    match std::fs::metadata(&staged_wal) {
        Ok(meta) if meta.len() > 0 => return Err(SplitError::StagedWal),
        Ok(_) => std::fs::remove_file(&staged_wal)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    remove_if_present(&format!("{staged}-shm"))?;
    std::fs::rename(&staged, memory)?;
    Ok(())
}

fn remove_if_present(path: &str) -> Result<(), SplitError> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

/// SQLite applies a `-wal` it finds next to a database file it opens, so an
/// orphan WAL with frames would replay old pages into the new memory
/// database. The WAL is checked before the shm is touched, so a refusal
/// leaves every file as it was.
fn clear_orphan_sidecars(memory: &Utf8Path) -> Result<(), SplitError> {
    let wal = format!("{memory}-wal");
    match std::fs::metadata(&wal) {
        Ok(meta) if meta.len() > 0 => return Err(SplitError::StaleWal),
        Ok(_) => std::fs::remove_file(&wal)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    remove_if_present(&format!("{memory}-shm"))?;
    remove_if_present(&format!("{memory}-journal"))
}

fn lock(path: &Utf8Path) -> Result<std::fs::File, SplitError> {
    crate::infra::fs::lock_exclusive(path)
        .map_err(|error| SplitError::Io(std::io::Error::other(error)))
}

/// Folds the WAL into the main file and leaves WAL mode before renaming:
/// renaming a database another connection has open, or separating a WAL
/// from its database, is how SQLite files get corrupted. Leaving WAL mode
/// needs the only connection to the file, so it doubles as the busy check.
fn move_legacy(memory: &Utf8Path, graph: &Utf8Path) -> Result<(), SplitError> {
    {
        let conn = Connection::open(memory.as_std_path())?;
        conn.busy_timeout(Duration::from_millis(250))?;
        let busy: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
        if busy != 0 {
            return Err(SplitError::Busy);
        }
        let mode: String =
            match conn.query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0)) {
                Err(rusqlite::Error::SqliteFailure(error, _))
                    if error.code == rusqlite::ErrorCode::DatabaseBusy =>
                {
                    return Err(SplitError::Busy);
                }
                result => result?,
            };
        if !mode.eq_ignore_ascii_case("delete") {
            return Err(SplitError::Busy);
        }
    }
    std::fs::rename(memory, graph)?;
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/split.rs"]
mod tests;
