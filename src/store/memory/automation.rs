//! `automation_decisions` and `automation_decision_texts`: the human's answers
//! to proposals to automate a recurring request.
//!
//! Personal data: no reindex touches it. `ivar automation decide`, `renew` and
//! `undo` write it, the retention sweep maintains it, and `scan`, `decisions`
//! and `doctor` only read it.

use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::automation::{Decision, DecisionKind, Form};

/// What a new decision is made of.
#[derive(Debug, Clone)]
pub struct NewDecision<'a> {
    pub kind: DecisionKind,
    pub intent: &'a str,
    pub form: Option<Form>,
    pub artifact: Option<&'a str>,
    pub until: Option<&'a str>,
    pub texts: &'a BTreeSet<String>,
}

const COLUMNS: &str = "id, kind, intent, form, artifact, until, created_at, updated_at";

fn row_to_decision(row: &rusqlite::Row<'_>) -> rusqlite::Result<Decision> {
    let corrupt = |column: usize, value: String| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            format!("unknown value `{value}`").into(),
        )
    };
    let kind: String = row.get(1)?;
    let form: Option<String> = row.get(3)?;
    Ok(Decision {
        id: row.get(0)?,
        kind: DecisionKind::parse(&kind).ok_or_else(|| corrupt(1, kind))?,
        intent: row.get(2)?,
        form: form
            .map(|form| Form::parse(&form).ok_or_else(|| corrupt(3, form)))
            .transpose()?,
        artifact: row.get(4)?,
        until: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        texts: BTreeSet::new(),
    })
}

fn texts_of(conn: &Connection, id: i64) -> rusqlite::Result<BTreeSet<String>> {
    conn.prepare_cached("SELECT normalized FROM automation_decision_texts WHERE decision_id = ?1")?
        .query_map([id], |row| row.get(0))?
        .collect()
}

/// Every decision, oldest first, with its texts.
///
/// # Errors
///
/// When the query fails.
pub fn list(conn: &Connection) -> rusqlite::Result<Vec<Decision>> {
    let mut decisions = conn
        .prepare(&format!(
            "SELECT {COLUMNS} FROM automation_decisions ORDER BY id"
        ))?
        .query_map([], row_to_decision)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for decision in &mut decisions {
        decision.texts = texts_of(conn, decision.id)?;
    }
    Ok(decisions)
}

/// The decision `id`, with its texts.
///
/// # Errors
///
/// When the query fails.
pub fn get(conn: &Connection, id: i64) -> rusqlite::Result<Option<Decision>> {
    let found = conn
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM automation_decisions WHERE id = ?1"
        ))?
        .query_row([id], row_to_decision)
        .optional()?;
    found
        .map(|mut decision| {
            decision.texts = texts_of(conn, id)?;
            Ok(decision)
        })
        .transpose()
}

/// Stores a decision and its texts; the caller wraps it in a transaction when
/// the two must land together. Returns the new id.
///
/// # Errors
///
/// When an insert fails.
pub fn insert(conn: &Connection, new: &NewDecision<'_>, now: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO automation_decisions
           (kind, intent, form, artifact, until, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![
            new.kind.as_str(),
            new.intent,
            new.form.map(Form::as_str),
            new.artifact,
            new.until,
            now
        ],
    )?;
    let id = conn.last_insert_rowid();
    add_texts(conn, id, new.texts)?;
    Ok(id)
}

/// Adds `texts` to decision `id`; a text it already holds is kept once.
///
/// # Errors
///
/// When an insert fails.
pub fn add_texts(conn: &Connection, id: i64, texts: &BTreeSet<String>) -> rusqlite::Result<usize> {
    let mut stmt = conn.prepare_cached(
        "INSERT OR IGNORE INTO automation_decision_texts (decision_id, normalized) VALUES (?1, ?2)",
    )?;
    let mut added = 0;
    for text in texts {
        added += stmt.execute(params![id, text])?;
    }
    Ok(added)
}

/// Rewrites `updated_at` of decision `id`. `false` when it does not exist.
///
/// # Errors
///
/// When the update fails.
pub fn touch(conn: &Connection, id: i64, now: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "UPDATE automation_decisions SET updated_at = ?2 WHERE id = ?1",
        params![id, now],
    )? > 0)
}

/// Deletes decision `id` and its texts. `false` when it does not exist.
///
/// # Errors
///
/// When a delete fails.
pub fn delete(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    conn.execute(
        "DELETE FROM automation_decision_texts WHERE decision_id = ?1",
        [id],
    )?;
    Ok(conn.execute("DELETE FROM automation_decisions WHERE id = ?1", [id])? > 0)
}
