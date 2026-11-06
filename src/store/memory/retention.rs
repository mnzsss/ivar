//! Retention of `session.*` observations. A session is kept or deleted whole,
//! judged by its newest observation; every other event name is never touched.
//! Request fingerprints are kept by date, 90 days.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use super::db::MemoryDb;
use crate::domain::session::rfc3339_from_epoch_secs;

pub const SESSION_EVENTS: [&str; 4] = [
    "session.started",
    "session.prompted",
    "session.tool",
    "session.ended",
];
pub const SESSION_RETENTION_DAYS: u64 = 30;
/// Request fingerprints outlive the observations they came from.
pub const FINGERPRINT_RETENTION_DAYS: u64 = 90;

fn session_events_in_list() -> String {
    SESSION_EVENTS
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(",")
}

fn expired_session_keys_sql(cutoff: &str) -> String {
    let events = session_events_in_list();
    format!(
        "SELECT session_id, native_session_id FROM observations
          WHERE source_event IN ({events})
          GROUP BY session_id, native_session_id
          HAVING max(occurred_at) < {cutoff}"
    )
}

fn delete_batch_sql() -> String {
    let events = session_events_in_list();
    let expired = expired_session_keys_sql("?1");
    format!(
        "DELETE FROM observations WHERE id IN (
    SELECT id FROM observations
    WHERE source_event IN ({events})
      AND (session_id, native_session_id) IN ({expired})
    LIMIT ?2)"
    )
}

/// SQL condition, for rows aliased `o`, that holds for every row the sweep
/// would keep at the cutoff bound to `cutoff`.
#[must_use]
pub fn retained_sql(cutoff: &str) -> String {
    let events = session_events_in_list();
    let expired = expired_session_keys_sql(cutoff);
    format!(
        "NOT (o.source_event IN ({events})
          AND (o.session_id, o.native_session_id) IN ({expired}))"
    )
}

fn oldest_session_sql() -> String {
    let events = session_events_in_list();
    format!(
        "SELECT min(last) FROM (
    SELECT max(occurred_at) AS last FROM observations
    WHERE source_event IN ({events})
    GROUP BY session_id, native_session_id)"
    )
}

/// The `occurred_at` text that separates rows older than `days` from newer
/// ones at `now_epoch_secs`.
#[must_use]
pub fn cutoff_before(now_epoch_secs: u64, days: u64) -> String {
    rfc3339_from_epoch_secs(now_epoch_secs.saturating_sub(days * 86_400) as f64)
}

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
        .execute(&delete_batch_sql(), rusqlite::params![cutoff, batch])
}

/// The UTC date (`YYYY-MM-DD`) `days` before `now_epoch_secs`: fingerprints
/// with an earlier `day` are expired.
#[must_use]
pub fn cutoff_day_before(now_epoch_secs: u64, days: u64) -> String {
    cutoff_before(now_epoch_secs, days)
        .chars()
        .take(10)
        .collect()
}

/// Deletes the fingerprints of prompts made before the UTC date `cutoff_day`.
///
/// # Errors
///
/// When `SQLite` fails.
pub fn delete_expired_fingerprints(db: &MemoryDb, cutoff_day: &str) -> rusqlite::Result<usize> {
    db.conn().execute(
        "DELETE FROM request_fingerprints WHERE day < ?1",
        [cutoff_day],
    )
}

#[must_use]
pub fn oldest_session_last_observation(path: &Path) -> Option<String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    conn.query_row(&oldest_session_sql(), [], |row| {
        row.get::<_, Option<String>>(0)
    })
    .ok()
    .flatten()
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/retention.rs"]
mod tests;
