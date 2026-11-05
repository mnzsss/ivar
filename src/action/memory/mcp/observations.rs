use serde_json::{Value, json};

use super::to_text;
use crate::action::memory::drain::drain;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;
use crate::store::memory::observations::{
    ObservationRow, recent_observations, session_observations,
};

const RECENT_DEFAULT: usize = 20;
const SESSION_DEFAULT: usize = 50;
const MAX_LIMIT: usize = 100;

pub(super) fn recent(layout: &Layout, args: &Value) -> Result<String, String> {
    let _ = drain(layout);
    let limit = bounded_limit(args, RECENT_DEFAULT);
    let db = open(layout)?;
    let rows = recent_observations(
        &db,
        limit,
        args.get("source_event").and_then(Value::as_str),
        args.get("feature").and_then(Value::as_str),
    )
    .map_err(|error| error.to_string())?;
    to_text(&json!({ "observations": rows_json(&rows) }))
}

pub(super) fn session(layout: &Layout, args: &Value) -> Result<String, String> {
    let session = args
        .get("session")
        .and_then(Value::as_str)
        .ok_or("`memory_read_session_observations` needs `session`")?;
    let _ = drain(layout);
    let limit = bounded_limit(args, SESSION_DEFAULT);
    let after = args.get("cursor").and_then(Value::as_i64);
    let db = open(layout)?;
    let mut rows =
        session_observations(&db, session, limit + 1, after).map_err(|error| error.to_string())?;
    let more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = rows.last().filter(|_| more).map(|row| row.id);
    to_text(&json!({ "observations": rows_json(&rows), "next_cursor": next_cursor }))
}

fn bounded_limit(args: &Value, default: usize) -> usize {
    args.get("limit")
        .and_then(Value::as_u64)
        .map_or(default, |n| {
            usize::try_from(n).map_or(MAX_LIMIT, |n| n.min(MAX_LIMIT))
        })
}

fn open(layout: &Layout) -> Result<MemoryDb, String> {
    MemoryDb::open(layout.memory_db().as_std_path()).map_err(|error| error.to_string())
}

fn rows_json(rows: &[ObservationRow]) -> Vec<Value> {
    rows.iter()
        .map(|row| {
            json!({
                "id": row.id,
                "source_event": row.source_event,
                "occurred_at": row.occurred_at,
                "session_id": row.session_id,
                "feature": row.feature,
                "summary": row.summary,
                "body": row.body,
            })
        })
        .collect()
}
