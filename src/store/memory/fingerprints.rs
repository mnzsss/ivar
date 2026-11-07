//! `request_fingerprints`: one normalized fingerprint per `User` prompt.
//!
//! The rows are written in the transaction that stores the prompt's
//! observation (and, once, by the migration that created the table) and kept
//! [`FINGERPRINT_RETENTION_DAYS`] by the sweep. `ivar automation scan` reads them.

use std::collections::BTreeSet;

use rusqlite::{Connection, params};

use crate::domain::fingerprint::normalize;
use crate::domain::observation::{Origin, SourceEvent};
use crate::domain::repetition::Fingerprint;
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

/// Every fingerprint of a prompt made on `since_day` (a `YYYY-MM-DD` date) or
/// later, oldest first.
///
/// # Errors
///
/// When the query fails.
pub fn since(conn: &Connection, since_day: &str) -> rusqlite::Result<Vec<Fingerprint>> {
    conn.prepare(
        "SELECT id, observation_id, session_id, feature, day, normalized
           FROM request_fingerprints WHERE day >= ?1 ORDER BY id",
    )?
    .query_map([since_day], |row| {
        Ok(Fingerprint {
            id: row.get(0)?,
            observation_id: row.get(1)?,
            session_id: row.get(2)?,
            feature: row.get(3)?,
            day: row.get(4)?,
            normalized: row.get(5)?,
        })
    })?
    .collect()
}

/// The `OR` query that matches a text sharing any of `tokens`; `None` when no
/// token has a letter or a digit for the index to hold. A placeholder such as
/// `<path>` is searched as the word `path`, which only widens the match.
fn any_token(tokens: &BTreeSet<String>) -> Option<String> {
    let terms: Vec<String> = tokens
        .iter()
        .map(|token| {
            token
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
        })
        .filter(|term| !term.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|term| format!("\"{term}\""))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

/// The distinct normalized texts, among the prompts made on `since_day` or
/// later, that share at least one token with `tokens`. Answered by the
/// full-text index; every text that is similar to `tokens` is among them, and
/// so are many that are not.
///
/// # Errors
///
/// When the query fails.
pub fn sharing_a_token(
    conn: &Connection,
    tokens: &BTreeSet<String>,
    since_day: &str,
) -> rusqlite::Result<Vec<String>> {
    let Some(expression) = any_token(tokens) else {
        return Ok(Vec::new());
    };
    conn.prepare_cached(
        "SELECT DISTINCT f.normalized
           FROM request_fingerprints_fts
           JOIN request_fingerprints f ON f.id = request_fingerprints_fts.rowid
          WHERE request_fingerprints_fts MATCH ?1 AND f.day >= ?2",
    )?
    .query_map(params![expression, since_day], |row| row.get(0))?
    .collect()
}

/// The ids, among those of `fingerprints`, whose observation is still stored.
/// The sweep deletes an observation after 30 days and SQLite can reuse its id,
/// so an id counts only when the row it names is the same user prompt.
///
/// # Errors
///
/// When the query fails.
pub fn stored_observations(
    conn: &Connection,
    fingerprints: &[&Fingerprint],
) -> rusqlite::Result<BTreeSet<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT EXISTS(SELECT 1 FROM observations
                        WHERE id = ?1 AND session_id = ?2 AND source_event = ?3 AND origin = ?4)",
    )?;
    let mut found = BTreeSet::new();
    for fingerprint in fingerprints {
        let stored: bool = stmt.query_row(
            params![
                fingerprint.observation_id,
                fingerprint.session_id,
                SourceEvent::SessionPrompted.as_str(),
                Origin::User.as_str()
            ],
            |row| row.get(0),
        )?;
        if stored {
            found.insert(fingerprint.observation_id);
        }
    }
    Ok(found)
}
