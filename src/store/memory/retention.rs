//! Retention of `session.*` observations. A session is kept or deleted whole,
//! judged by its newest observation; every other event name is never touched.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use super::db::MemoryDb;

pub const SESSION_EVENTS: [&str; 4] = [
    "session.started",
    "session.prompted",
    "session.tool",
    "session.ended",
];
pub const SESSION_RETENTION_DAYS: u64 = 30;

const DELETE_BATCH: &str = "
DELETE FROM observations WHERE id IN (
    SELECT id FROM observations
    WHERE source_event IN ('session.started','session.prompted','session.tool','session.ended')
      AND (session_id, native_session_id) IN (
          SELECT session_id, native_session_id FROM observations
          WHERE source_event IN ('session.started','session.prompted','session.tool','session.ended')
          GROUP BY session_id, native_session_id
          HAVING max(occurred_at) < ?1)
    LIMIT ?2)";

const OLDEST_SESSION: &str = "
SELECT min(last) FROM (
    SELECT max(occurred_at) AS last FROM observations
    WHERE source_event IN ('session.started','session.prompted','session.tool','session.ended')
    GROUP BY session_id, native_session_id)";

/// Deletes up to `batch` rows of sessions whose newest observation is older
/// than `cutoff`. `occurred_at` is compared as text, which is only correct
/// because every writer uses the fixed-width form of `rfc3339_from_epoch_secs`.
///
/// # Errors
///
/// When `SQLite` fails.
pub fn delete_expired_sessions(
    db: &mut MemoryDb,
    cutoff: &str,
    batch: usize,
) -> rusqlite::Result<usize> {
    db.conn()
        .execute(DELETE_BATCH, rusqlite::params![cutoff, batch])
}

#[must_use]
pub fn oldest_session_last_observation(path: &Path) -> Option<String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    conn.query_row(OLDEST_SESSION, [], |row| row.get::<_, Option<String>>(0))
        .ok()
        .flatten()
}
