//! `request_fingerprints`: one normalized fingerprint per `User` prompt.
//!
//! The rows are written in the transaction that stores the prompt's
//! observation (and, once, by the migration that created the table) and kept
//! [`FINGERPRINT_RETENTION_DAYS`] by the sweep. Nothing reads them yet.

use rusqlite::{Connection, params};

use crate::domain::fingerprint::normalize;
use crate::domain::observation::{Origin, SourceEvent};
use crate::domain::session::rfc3339_now;

/// Only a prompt the user typed has a fingerprint; a `harness` prompt (a hook
/// or tool injecting text) never does.
#[must_use]
pub fn is_fingerprinted(source_event: &str, origin: &str) -> bool {
    source_event == SourceEvent::SessionPrompted.as_str() && origin == Origin::User.as_str()
}

/// The UTC date of an RFC 3339 `occurred_at`, which every writer emits in the
/// fixed-width `Z` form.
fn day_of(occurred_at: &str) -> &str {
    occurred_at.get(..10).unwrap_or(occurred_at)
}

/// Inserts the fingerprint of the prompt stored as observation
/// `observation_id`. `body` is the sanitized observation body.
///
/// # Errors
///
/// When the insert fails.
pub fn insert(
    conn: &Connection,
    observation_id: i64,
    session_id: &str,
    feature: &str,
    occurred_at: &str,
    body: &str,
) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "INSERT INTO request_fingerprints
           (observation_id, session_id, feature, day, normalized, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?
    .execute(params![
        observation_id,
        session_id,
        feature,
        day_of(occurred_at),
        normalize(body),
        rfc3339_now()
    ])?;
    Ok(())
}

/// One fingerprint for every retained `User` prompt.
pub(super) fn backfill(conn: &Connection) -> rusqlite::Result<()> {
    type Row = (i64, String, String, String, String);
    let rows: Vec<Row> = {
        let mut stmt = conn.prepare(
            "SELECT id, session_id, feature, occurred_at, body FROM observations
              WHERE source_event = ?1 AND origin = ?2 ORDER BY id",
        )?;
        stmt.query_map(
            params![SourceEvent::SessionPrompted.as_str(), Origin::User.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?
        .collect::<Result<_, _>>()?
    };
    for (id, session_id, feature, occurred_at, body) in rows {
        insert(conn, id, &session_id, &feature, &occurred_at, &body)?;
    }
    Ok(())
}
