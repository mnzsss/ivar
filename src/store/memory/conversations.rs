//! Reads of `observations` grouped by provider conversation (`native_session_id`).

use super::db::MemoryDb;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationHead {
    pub native_session_id: String,
    pub session_id: String,
    pub first_at: String,
    pub last_at: String,
    pub last_id: i64,
    pub ended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRow {
    pub id: i64,
    pub source_event: String,
    pub occurred_at: String,
    pub session_id: String,
    pub native_session_id: String,
    pub provider: String,
    pub feature: String,
    pub repo: String,
    pub origin: String,
    pub importance: u8,
    pub body: String,
}

// `occurred_at` is compared as text: `rfc3339_from_epoch_secs` writes a fixed-width form.
const HEADS_SQL: &str = "
SELECT native_session_id, max(session_id), min(occurred_at), max(occurred_at), max(id),
       max(source_event = 'session.ended')
  FROM observations
 WHERE native_session_id != '' AND source_event LIKE 'session.%'
   AND (?2 IS NULL OR session_id = ?2)
 GROUP BY native_session_id
HAVING max(occurred_at) >= ?1
 ORDER BY native_session_id";

const ROW_COLUMNS: &str = "id, source_event, occurred_at, session_id, native_session_id,
    provider, feature, repo, origin, importance, body";

/// One head per conversation whose newest `session.*` row is at or after `since`.
///
/// # Errors
///
/// When `SQLite` fails.
pub fn conversation_heads(
    db: &MemoryDb,
    since: &str,
    session_id: Option<&str>,
) -> rusqlite::Result<Vec<ConversationHead>> {
    let mut stmt = db.conn().prepare(HEADS_SQL)?;
    stmt.query_map(rusqlite::params![since, session_id], |row| {
        Ok(ConversationHead {
            native_session_id: row.get(0)?,
            session_id: row.get(1)?,
            first_at: row.get(2)?,
            last_at: row.get(3)?,
            last_id: row.get(4)?,
            ended: row.get::<_, i64>(5)? != 0,
        })
    })?
    .collect()
}

/// Every row of one conversation, in id order.
///
/// # Errors
///
/// When `SQLite` fails.
pub fn conversation_rows(
    db: &MemoryDb,
    native_session_id: &str,
) -> rusqlite::Result<Vec<ConversationRow>> {
    rows(
        db,
        &format!("SELECT {ROW_COLUMNS} FROM observations WHERE native_session_id = ?1 ORDER BY id"),
        native_session_id,
    )
}

/// The `delivery.*` and `memory.*` rows of a session that carry no conversation.
///
/// # Errors
///
/// When `SQLite` fails.
pub fn unattached_rows(db: &MemoryDb, session_id: &str) -> rusqlite::Result<Vec<ConversationRow>> {
    rows(
        db,
        &format!(
            "SELECT {ROW_COLUMNS} FROM observations
              WHERE native_session_id = '' AND session_id = ?1 AND session_id != ''
                AND (source_event LIKE 'delivery.%' OR source_event LIKE 'memory.%')
              ORDER BY id"
        ),
        session_id,
    )
}

fn rows(db: &MemoryDb, sql: &str, key: &str) -> rusqlite::Result<Vec<ConversationRow>> {
    let mut stmt = db.conn().prepare(sql)?;
    stmt.query_map([key], |row| {
        Ok(ConversationRow {
            id: row.get(0)?,
            source_event: row.get(1)?,
            occurred_at: row.get(2)?,
            session_id: row.get(3)?,
            native_session_id: row.get(4)?,
            provider: row.get(5)?,
            feature: row.get(6)?,
            repo: row.get(7)?,
            origin: row.get(8)?,
            importance: row.get(9)?,
            body: row.get(10)?,
        })
    })?
    .collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/conversations.rs"]
mod tests;
