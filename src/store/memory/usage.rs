//! `memory_usage` and `memory_misses`: what `memory_query` was asked and what it failed to answer.

use rusqlite::{Connection, params};
use serde::Serialize;

use crate::domain::graph::truncate_for_storage;

pub const USAGE_RETENTION_DAYS: u64 = 30;

#[derive(Debug, Clone)]
pub struct QueryRecord {
    pub source: String,
    pub session: Option<String>,
    pub query: String,
    pub filters: String,
    pub page_count: usize,
    pub observation_count: usize,
    pub duration_ms: u64,
    pub error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryMissKind {
    Empty,
    Followup,
    Skipped,
}

impl MemoryMissKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Followup => "followup",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MemoryMiss {
    pub kind: MemoryMissKind,
    pub session: Option<String>,
    pub query: Option<String>,
    pub filters: Option<String>,
    pub pattern: Option<String>,
    pub usage_id: Option<i64>,
    pub fallback: bool,
}

#[derive(Debug, Clone)]
pub struct LastQuery {
    pub usage_id: i64,
    pub ts: i64,
    pub query: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct UsageSummary {
    pub queries_cli: usize,
    pub queries_mcp: usize,
    pub empty: usize,
    pub followup: usize,
    pub skipped: usize,
}

/// # Errors
///
/// When the insert fails.
pub fn record_query(conn: &Connection, record: &QueryRecord) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO memory_usage (ts, source, session, query, filters, page_count, observation_count, duration_ms, error)
         VALUES (unixepoch(), ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            record.source,
            record.session,
            truncate_for_storage(&record.query),
            record.filters,
            i64::try_from(record.page_count).unwrap_or(i64::MAX),
            i64::try_from(record.observation_count).unwrap_or(i64::MAX),
            i64::try_from(record.duration_ms).unwrap_or(i64::MAX),
            record.error
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// # Errors
///
/// When the insert fails.
pub fn record_miss(conn: &Connection, miss: &MemoryMiss) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO memory_misses (ts, session, kind, query, filters, pattern, usage_id, fallback)
         VALUES (unixepoch(), ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            miss.session,
            miss.kind.as_str(),
            miss.query,
            miss.filters,
            miss.pattern,
            miss.usage_id,
            miss.fallback
        ],
    )
    .map(|_| ())
}

/// # Errors
///
/// When the lookup fails.
pub fn last_query(conn: &Connection, session: &str) -> rusqlite::Result<Option<LastQuery>> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, query FROM memory_usage WHERE session = ?1 ORDER BY ts DESC, id DESC LIMIT 1",
    )?;
    let mut rows = stmt.query_map([session], |row| {
        Ok(LastQuery {
            usage_id: row.get(0)?,
            ts: row.get(1)?,
            query: row.get(2)?,
        })
    })?;
    rows.next().transpose()
}

/// # Errors
///
/// When the lookup fails.
pub fn has_followup_for(conn: &Connection, usage_id: i64) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM memory_misses WHERE kind = 'followup' AND usage_id = ?1)",
        [usage_id],
        |row| row.get(0),
    )
}

/// # Errors
///
/// When a count query fails.
pub fn summary(conn: &Connection, since_ts: i64) -> rusqlite::Result<UsageSummary> {
    let mut out = UsageSummary::default();
    let mut queries = conn.prepare(
        "SELECT source, count(*) FROM memory_usage WHERE ts >= ?1 AND source != 'hook' GROUP BY source",
    )?;
    for row in queries.query_map([since_ts], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, usize>(1)?))
    })? {
        match row? {
            (source, n) if source == "cli" => out.queries_cli = n,
            (source, n) if source == "mcp" => out.queries_mcp = n,
            _ => {}
        }
    }
    let mut misses =
        conn.prepare("SELECT kind, count(*) FROM memory_misses WHERE ts >= ?1 GROUP BY kind")?;
    for row in misses.query_map([since_ts], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, usize>(1)?))
    })? {
        match row? {
            (kind, n) if kind == "empty" => out.empty = n,
            (kind, n) if kind == "followup" => out.followup = n,
            (kind, n) if kind == "skipped" => out.skipped = n,
            _ => {}
        }
    }
    Ok(out)
}

/// # Errors
///
/// When a delete fails.
pub fn prune(conn: &Connection, before_ts: i64) -> rusqlite::Result<usize> {
    Ok(
        conn.execute("DELETE FROM memory_usage WHERE ts < ?1", [before_ts])?
            + conn.execute("DELETE FROM memory_misses WHERE ts < ?1", [before_ts])?,
    )
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/usage.rs"]
mod tests;
