//! Writing and counting rows of the `observations` table.

use rusqlite::Connection;

use super::db::MemoryDb;
use super::fingerprints;
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
            let new = stmt.execute(rusqlite::params![
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
            // A replayed event is ignored above, so it never gets a second fingerprint.
            if new == 1 && fingerprints::is_fingerprinted(&o.source_event, &o.origin) {
                fingerprints::insert(
                    &tx,
                    tx.last_insert_rowid(),
                    &o.session_id,
                    &o.feature,
                    &o.occurred_at,
                    &o.body,
                )?;
            }
            inserted += new;
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

/// One stored observation as the read tools show it; `summary` is the first
/// line of `body`, cut to 160 characters.
#[derive(Debug, Clone)]
pub struct ObservationRow {
    pub id: i64,
    pub source_event: String,
    pub occurred_at: String,
    pub session_id: String,
    pub feature: String,
    pub summary: String,
    pub body: String,
}

/// Newest observations first, optionally limited to a `source_event` prefix
/// and one feature.
///
/// # Errors
///
/// Returns the `SQLite` error when the query fails.
pub fn recent_observations(
    db: &MemoryDb,
    limit: usize,
    source_prefix: Option<&str>,
    feature: Option<&str>,
) -> rusqlite::Result<Vec<ObservationRow>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, source_event, occurred_at, session_id, feature, body FROM observations
         WHERE (?1 IS NULL OR substr(source_event, 1, length(?1)) = ?1)
           AND (?2 IS NULL OR feature = ?2)
         ORDER BY occurred_at DESC, id DESC LIMIT ?3",
    )?;
    stmt.query_map(
        rusqlite::params![
            source_prefix,
            feature,
            i64::try_from(limit).unwrap_or(i64::MAX)
        ],
        observation_row,
    )?
    .collect()
}

/// Observations of one session (matched on `session_id` or
/// `native_session_id`), oldest first, with `id` above `after_id`.
///
/// # Errors
///
/// Returns the `SQLite` error when the query fails.
pub fn session_observations(
    db: &MemoryDb,
    session: &str,
    limit: usize,
    after_id: Option<i64>,
) -> rusqlite::Result<Vec<ObservationRow>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, source_event, occurred_at, session_id, feature, body FROM observations
         WHERE (session_id = ?1 OR native_session_id = ?1) AND id > ?2
         ORDER BY id ASC LIMIT ?3",
    )?;
    stmt.query_map(
        rusqlite::params![
            session,
            after_id.unwrap_or(0),
            i64::try_from(limit).unwrap_or(i64::MAX)
        ],
        observation_row,
    )?
    .collect()
}

/// Whether the session prompted or ran a tool, matched on `session_id` or
/// `native_session_id`.
///
/// # Errors
///
/// Returns the `SQLite` error when the query fails.
pub fn session_has_activity(conn: &Connection, session: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM observations
           WHERE (session_id = ?1 OR native_session_id = ?1)
             AND source_event IN ('session.prompted', 'session.tool'))",
        [session],
        |row| row.get(0),
    )
}

/// The bodies of the `session.tool` observations of one session (matched on
/// `session_id`), oldest first. Read-only.
///
/// # Errors
///
/// Returns the `SQLite` error when the query fails.
pub fn session_tool_bodies(conn: &Connection, session: &str) -> rusqlite::Result<Vec<String>> {
    conn.prepare_cached(
        "SELECT body FROM observations
          WHERE session_id = ?1 AND source_event = 'session.tool' ORDER BY id",
    )?
    .query_map([session], |row| row.get(0))?
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureObservation {
    pub id: i64,
    pub source_event: String,
    pub occurred_at: String,
    pub session_id: String,
    pub feature: String,
    pub importance: u8,
    pub body: String,
}

/// The rows of `features`, plus every row of a session that touched one of
/// them: a discovery session keeps its id when it converts to the feature.
///
/// # Errors
///
/// Returns the `SQLite` error when the query fails.
pub fn feature_observations(
    db: &MemoryDb,
    features: &[String],
) -> rusqlite::Result<Vec<FeatureObservation>> {
    feature_observations_on(db.conn(), features)
}

/// [`feature_observations`] on a bare connection, for a read that opened the
/// database read-only.
///
/// # Errors
///
/// Returns the `SQLite` error when the query fails.
pub fn feature_observations_on(
    conn: &Connection,
    features: &[String],
) -> rusqlite::Result<Vec<FeatureObservation>> {
    if features.is_empty() {
        return Ok(Vec::new());
    }
    let list = serde_json::to_string(features).unwrap_or_else(|_| "[]".to_owned());
    let mut stmt = conn.prepare(
        "WITH wanted(name) AS (SELECT value FROM json_each(?1)),
              sessions(id) AS (
                SELECT DISTINCT session_id FROM observations
                 WHERE session_id <> '' AND feature IN (SELECT name FROM wanted))
         SELECT id, source_event, occurred_at, session_id, feature, importance, body
           FROM observations
          WHERE feature IN (SELECT name FROM wanted)
             OR session_id IN (SELECT id FROM sessions)
          ORDER BY occurred_at, id",
    )?;
    stmt.query_map([list], |row| {
        Ok(FeatureObservation {
            id: row.get(0)?,
            source_event: row.get(1)?,
            occurred_at: row.get(2)?,
            session_id: row.get(3)?,
            feature: row.get(4)?,
            importance: row.get(5)?,
            body: row.get(6)?,
        })
    })?
    .collect()
}

fn observation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ObservationRow> {
    let body: String = row.get(5)?;
    Ok(ObservationRow {
        id: row.get(0)?,
        source_event: row.get(1)?,
        occurred_at: row.get(2)?,
        session_id: row.get(3)?,
        feature: row.get(4)?,
        summary: body
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(160)
            .collect(),
        body,
    })
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/observations.rs"]
mod tests;
