//! `handoffs` and `messages`: the coordination queues in `memory.db`.
//!
//! Every claim is one conditional `UPDATE` checked with `changes() == 1`.
//! Visibility is not decided here; callers filter rows with `Viewer::sees`.

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::domain::coordination::{
    Address, Handoff, HandoffState, Message, MessageState, NewHandoff, NewMessage, Owner, Queue,
};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Expired {
    pub handoffs: Vec<String>,
    pub messages: usize,
}

const HANDOFF_COLUMNS: &str = "id, queue, to_kind, to_value, owner_key, owner_provider, summary,
    pending_tasks, open_decisions, modified_files, state, created_at, expires_at,
    accepted_by, accepted_at";

const MESSAGE_COLUMNS: &str = "id, queue, to_kind, to_value, sender_key, sender_provider, subject,
    body, state, created_at, expires_at, popped_by, popped_at";

fn address_columns(address: Option<&Address>) -> (&str, &str) {
    address.map_or(("", ""), |address| (address.kind(), address.value()))
}

/// `Some(None)` is an unaddressed row; `None` is a row whose address cannot
/// be read back, including a known kind with a blank value.
fn address_from_columns(kind: &str, value: &str) -> Option<Option<Address>> {
    match (kind.is_empty(), value.trim().is_empty()) {
        (true, true) => Some(None),
        (false, false) => Address::from_columns(kind, value).map(Some),
        _ => None,
    }
}

fn list_column(items: &[String]) -> String {
    serde_json::to_string(items).unwrap_or_else(|_| "[]".to_owned())
}

fn list_from_column(value: &str) -> Option<Vec<String>> {
    serde_json::from_str(value).ok()
}

fn handoff_from_row(row: &Row<'_>) -> rusqlite::Result<Option<Handoff>> {
    let queue: String = row.get(1)?;
    let to_kind: String = row.get(2)?;
    let to_value: String = row.get(3)?;
    let pending_tasks: String = row.get(7)?;
    let open_decisions: String = row.get(8)?;
    let modified_files: String = row.get(9)?;
    let state: String = row.get(10)?;
    let parsed = (|| {
        Some((
            Queue::from_column(&queue)?,
            address_from_columns(&to_kind, &to_value)?,
            list_from_column(&pending_tasks)?,
            list_from_column(&open_decisions)?,
            list_from_column(&modified_files)?,
            HandoffState::parse(&state)?,
        ))
    })();
    let Some((queue, address, pending_tasks, open_decisions, modified_files, state)) = parsed
    else {
        return Ok(None);
    };
    Ok(Some(Handoff {
        id: row.get(0)?,
        queue,
        address,
        owner: Owner {
            key: row.get(4)?,
            provider: row.get(5)?,
        },
        summary: row.get(6)?,
        pending_tasks,
        open_decisions,
        modified_files,
        created_at: row.get(11)?,
        expires_at: row.get(12)?,
        state,
        accepted_by: row.get(13)?,
        accepted_at: row.get(14)?,
    }))
}

fn message_from_row(row: &Row<'_>) -> rusqlite::Result<Option<Message>> {
    let queue: String = row.get(1)?;
    let to_kind: String = row.get(2)?;
    let to_value: String = row.get(3)?;
    let state: String = row.get(8)?;
    let (Some(queue), Some(address), Some(state)) = (
        Queue::from_column(&queue),
        address_from_columns(&to_kind, &to_value),
        MessageState::parse(&state),
    ) else {
        return Ok(None);
    };
    Ok(Some(Message {
        id: row.get(0)?,
        queue,
        address,
        sender: Owner {
            key: row.get(4)?,
            provider: row.get(5)?,
        },
        subject: row.get(6)?,
        body: row.get(7)?,
        created_at: row.get(9)?,
        expires_at: row.get(10)?,
        state,
        popped_by: row.get(11)?,
        popped_at: row.get(12)?,
    }))
}

fn readable<T>(
    conn: &Connection,
    sql: &str,
    now: i64,
    from_row: fn(&Row<'_>) -> rusqlite::Result<Option<T>>,
) -> rusqlite::Result<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([now], from_row)?;
    let mut items = Vec::new();
    for row in rows {
        items.extend(row?);
    }
    Ok(items)
}

/// # Errors
/// Returns the `SQLite` error, including a duplicate id.
pub fn insert_handoff(conn: &Connection, handoff: &NewHandoff) -> rusqlite::Result<()> {
    let (to_kind, to_value) = address_columns(handoff.address.as_ref());
    conn.execute(
        "INSERT INTO handoffs (id, queue, to_kind, to_value, owner_key, owner_provider, summary,
            pending_tasks, open_decisions, modified_files, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            handoff.id,
            handoff.queue.column(),
            to_kind,
            to_value,
            handoff.owner.key,
            handoff.owner.provider,
            handoff.summary,
            list_column(&handoff.pending_tasks),
            list_column(&handoff.open_decisions),
            list_column(&handoff.modified_files),
            handoff.created_at,
            handoff.expires_at,
        ],
    )?;
    Ok(())
}

/// # Errors
/// Returns the `SQLite` error.
pub fn open_handoffs(conn: &Connection, now: i64) -> rusqlite::Result<Vec<Handoff>> {
    readable(
        conn,
        &format!(
            "SELECT {HANDOFF_COLUMNS} FROM handoffs
              WHERE state = 'open' AND expires_at > ?1
              ORDER BY created_at DESC, id DESC"
        ),
        now,
        handoff_from_row,
    )
}

/// # Errors
/// Returns the `SQLite` error.
pub fn get_handoff(conn: &Connection, id: &str) -> rusqlite::Result<Option<Handoff>> {
    conn.query_row(
        &format!("SELECT {HANDOFF_COLUMNS} FROM handoffs WHERE id = ?1"),
        [id],
        handoff_from_row,
    )
    .optional()
    .map(Option::flatten)
}

/// # Errors
/// Returns the `SQLite` error.
pub fn claim_handoff(conn: &Connection, id: &str, by: &str, now: i64) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE handoffs SET state = 'accepted', accepted_by = ?2, accepted_at = ?3
          WHERE id = ?1 AND state = 'open' AND expires_at > ?3",
        params![id, by, now],
    )?;
    Ok(changed == 1)
}

/// `None` as `owner_key` cancels whoever owns the handoff.
///
/// # Errors
/// Returns the `SQLite` error.
pub fn cancel_handoff(
    conn: &Connection,
    id: &str,
    owner_key: Option<&str>,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE handoffs SET state = 'cancelled'
          WHERE id = ?1 AND state = 'open' AND (?2 IS NULL OR owner_key = ?2)",
        params![id, owner_key],
    )?;
    Ok(changed == 1)
}

/// # Errors
/// Returns the `SQLite` error.
pub fn owner_has_handoff(conn: &Connection, owner_key: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM handoffs WHERE owner_key = ?1)",
        [owner_key],
        |row| row.get(0),
    )
}

/// # Errors
/// Returns the `SQLite` error, including a duplicate id.
pub fn insert_message(conn: &Connection, message: &NewMessage) -> rusqlite::Result<()> {
    let (to_kind, to_value) = address_columns(message.address.as_ref());
    conn.execute(
        "INSERT INTO messages (id, queue, to_kind, to_value, sender_key, sender_provider,
            subject, body, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            message.id,
            message.queue.column(),
            to_kind,
            to_value,
            message.sender.key,
            message.sender.provider,
            message.subject,
            message.body,
            message.created_at,
            message.expires_at,
        ],
    )?;
    Ok(())
}

/// # Errors
/// Returns the `SQLite` error.
pub fn pending_messages(conn: &Connection, now: i64) -> rusqlite::Result<Vec<Message>> {
    readable(
        conn,
        &format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages
              WHERE state = 'pending' AND expires_at > ?1
              ORDER BY created_at ASC, id ASC"
        ),
        now,
        message_from_row,
    )
}

/// # Errors
/// Returns the `SQLite` error.
pub fn get_message(conn: &Connection, id: &str) -> rusqlite::Result<Option<Message>> {
    conn.query_row(
        &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"),
        [id],
        message_from_row,
    )
    .optional()
    .map(Option::flatten)
}

/// # Errors
/// Returns the `SQLite` error.
pub fn pop_message(conn: &Connection, id: &str, by: &str, now: i64) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE messages SET state = 'popped', popped_by = ?2, popped_at = ?3
          WHERE id = ?1 AND state = 'pending' AND expires_at > ?3",
        params![id, by, now],
    )?;
    Ok(changed == 1)
}

/// `None` as `id` cancels every pending message of the sender.
///
/// # Errors
/// Returns the `SQLite` error.
pub fn cancel_messages(
    conn: &Connection,
    sender_key: &str,
    id: Option<&str>,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE messages SET state = 'cancelled'
          WHERE sender_key = ?1 AND state = 'pending' AND (?2 IS NULL OR id = ?2)",
        params![sender_key, id],
    )
}

/// # Errors
/// Returns the `SQLite` error.
pub fn expire_overdue(conn: &Connection, now: i64) -> rusqlite::Result<Expired> {
    let mut stmt = conn.prepare(
        "UPDATE handoffs SET state = 'expired'
          WHERE state = 'open' AND expires_at <= ?1 RETURNING id",
    )?;
    let handoffs = stmt
        .query_map([now], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    let messages = conn.execute(
        "UPDATE messages SET state = 'expired'
          WHERE state = 'pending' AND expires_at <= ?1",
        [now],
    )?;
    Ok(Expired { handoffs, messages })
}

/// Deletes terminal rows aged before `before` from both tables.
///
/// # Errors
/// Returns the `SQLite` error.
pub fn prune_terminal(conn: &Connection, before: i64) -> rusqlite::Result<usize> {
    let handoffs = conn.execute(
        "DELETE FROM handoffs
          WHERE state <> 'open' AND COALESCE(accepted_at, expires_at) < ?1",
        [before],
    )?;
    let messages = conn.execute(
        "DELETE FROM messages
          WHERE state <> 'pending' AND COALESCE(popped_at, expires_at) < ?1",
        [before],
    )?;
    Ok(handoffs + messages)
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/coordination.rs"]
mod tests;
