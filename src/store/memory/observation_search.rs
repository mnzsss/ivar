//! Full-text search over captured observations.

use super::db::MemoryDb;
use super::retention::retained_sql;

#[derive(Debug, Clone)]
pub struct ObservationMatch {
    pub source_event: String,
    pub occurred_at: String,
    pub snippet: String,
}

/// Only observations the sweep would keep at `retention_cutoff` are returned.
///
/// # Errors
///
/// Returns the `SQLite` error when the query fails.
pub fn search(
    db: &MemoryDb,
    fts_query: &str,
    limit: usize,
    retention_cutoff: &str,
) -> rusqlite::Result<Vec<ObservationMatch>> {
    let retained = retained_sql("?3");
    let mut stmt = db.conn().prepare(&format!(
        "SELECT o.source_event, o.occurred_at, snippet(observations_fts, 1, '', '', '...', 24)
           FROM observations_fts
           JOIN observations o ON o.id = observations_fts.rowid
          WHERE observations_fts MATCH ?1 AND {retained}
          ORDER BY bm25(observations_fts)
          LIMIT ?2"
    ))?;
    stmt.query_map(
        rusqlite::params![
            fts_query,
            i64::try_from(limit).unwrap_or(i64::MAX),
            retention_cutoff
        ],
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
