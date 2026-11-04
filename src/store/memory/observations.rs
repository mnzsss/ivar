//! Writing and counting rows of the `observations` table.

use super::db::MemoryDb;
use crate::domain::observation::Observation;
use crate::domain::sanitize::Sanitized;

/// Inserts `batch` in one transaction and returns how many rows were new; an
/// `event_id` already stored is ignored, which makes a replayed spool harmless.
///
/// # Errors
///
/// Returns the `SQLite` error; nothing from the batch is stored then.
pub fn insert_batch(
    db: &mut MemoryDb,
    batch: &[Sanitized<Observation>],
) -> rusqlite::Result<usize> {
    let tx = db.conn_mut().transaction()?;
    let mut inserted = 0;
    {
        let mut stmt = tx.prepare(
            "INSERT OR IGNORE INTO observations
               (event_id, source_event, occurred_at, session_id, native_session_id, provider,
                ivar_version, feature, parent_feature, repo, origin, importance, attempt_id, body)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        )?;
        for item in batch {
            let o = item.as_observation();
            inserted += stmt.execute(rusqlite::params![
                o.event_id,
                o.source_event,
                o.occurred_at,
                o.session_id,
                o.native_session_id,
                o.provider,
                o.ivar_version,
                o.feature,
                o.parent_feature,
                o.repo,
                o.origin,
                o.importance.get(),
                o.attempt_id,
                o.body
            ])?;
        }
    }
    tx.commit()?;
    Ok(inserted)
}

/// Row count per `source_event`, ordered by name.
///
/// # Errors
///
/// Returns the `SQLite` error when the query fails.
pub fn count_by_source_event(db: &MemoryDb) -> rusqlite::Result<Vec<(String, i64)>> {
    let mut stmt = db.conn().prepare(
        "SELECT source_event, count(*) FROM observations GROUP BY source_event ORDER BY source_event",
    )?;
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/observations.rs"]
mod tests;
