//! Full-text search over captured observations.

use super::db::MemoryDb;

#[derive(Debug, Clone)]
pub struct ObservationMatch {
    pub source_event: String,
    pub occurred_at: String,
    pub snippet: String,
}

/// # Errors
///
/// Returns the `SQLite` error when the query fails.
pub fn search(
    db: &MemoryDb,
    fts_query: &str,
    limit: usize,
) -> rusqlite::Result<Vec<ObservationMatch>> {
    let mut stmt = db.conn().prepare(
        "SELECT o.source_event, o.occurred_at, snippet(observations_fts, 1, '', '', '...', 24)
           FROM observations_fts
           JOIN observations o ON o.id = observations_fts.rowid
          WHERE observations_fts MATCH ?1
          ORDER BY bm25(observations_fts)
          LIMIT ?2",
    )?;
    stmt.query_map(
        rusqlite::params![fts_query, i64::try_from(limit).unwrap_or(i64::MAX)],
        |row| {
            Ok(ObservationMatch {
                source_event: row.get(0)?,
                occurred_at: row.get(1)?,
                snippet: row.get(2)?,
            })
        },
    )?
    .collect()
}
