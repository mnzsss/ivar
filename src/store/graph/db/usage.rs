use std::time::Duration;

use rusqlite::{OptionalExtension, params};

use super::GraphDb;
use super::types::{Result, now_timestamp};
use crate::domain::graph::{MissEvent, MissKind, MissRecord, UsageEvent, UsageSource, UsageStats};

pub(super) const USAGE_BUSY_TIMEOUT: Duration = Duration::from_millis(50);
const DEFAULT_BUSY_TIMEOUT: Duration = Duration::from_secs(10);

fn nearest_rank(sorted: &[u64], pct: u64) -> u64 {
    let rank = usize::try_from(
        (u64::try_from(sorted.len()).unwrap_or(u64::MAX) * pct)
            .div_ceil(100)
            .max(1),
    )
    .unwrap_or(usize::MAX);
    sorted.get(rank - 1).copied().unwrap_or(0)
}

struct UsageRow {
    command: String,
    source: String,
    ts: i64,
    duration_ms: u64,
    result_count: Option<i64>,
    error: bool,
}

struct UsageGroup {
    stats: UsageStats,
    durations: Vec<u64>,
}

impl GraphDb {
    /// Record one usage event.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDbError`] if the insert fails.
    pub fn record_usage(&self, event: &UsageEvent) -> Result<()> {
        // A usage write must never hold a query hostage to another session's lock.
        self.conn.busy_timeout(USAGE_BUSY_TIMEOUT)?;
        let inserted = self.conn.execute(
            "INSERT INTO usage (command, source, ts, duration_ms, result_count, error, session, query)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                event.command,
                event.source.as_str(),
                now_timestamp(),
                i64::try_from(event.duration_ms).unwrap_or(i64::MAX),
                event
                    .result_count
                    .map(|c| i64::try_from(c).unwrap_or(i64::MAX)),
                event.error,
                event.session,
                event.query,
            ],
        );
        let _ = self.conn.busy_timeout(DEFAULT_BUSY_TIMEOUT);
        inserted.map(|_| ()).map_err(Into::into)
    }

    /// Aggregated usage stats grouped by command and source, with p50/p95
    /// durations.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDbError`] if the query fails.
    pub fn usage_summary(&self) -> Result<Vec<UsageStats>> {
        let mut stmt = self.conn.prepare(
            "SELECT command, source, ts, duration_ms, result_count, error
             FROM usage WHERE source != 'hook' ORDER BY command, source, duration_ms",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(UsageRow {
                command: r.get(0)?,
                source: r.get(1)?,
                ts: r.get(2)?,
                duration_ms: u64::try_from(r.get::<_, i64>(3)?.max(0)).unwrap_or(0),
                result_count: r.get(4)?,
                error: r.get(5)?,
            })
        })?;

        let mut groups: Vec<UsageGroup> = Vec::new();
        for row in rows {
            let row = row?;
            let Ok(source) = UsageSource::try_from(row.source.as_str()) else {
                continue;
            };
            let same_group = groups
                .last()
                .is_some_and(|g| g.stats.command == row.command && g.stats.source == source);
            if !same_group {
                groups.push(UsageGroup {
                    stats: UsageStats {
                        command: row.command,
                        source,
                        count: 0,
                        last_used: 0,
                        empty_count: 0,
                        error_count: 0,
                        p50_ms: 0,
                        p95_ms: 0,
                    },
                    durations: Vec::new(),
                });
            }
            if let Some(group) = groups.last_mut() {
                group.stats.count += 1;
                group.stats.last_used = group.stats.last_used.max(row.ts);
                group.stats.empty_count += u64::from(row.result_count == Some(0));
                group.stats.error_count += u64::from(row.error);
                group.durations.push(row.duration_ms);
            }
        }

        Ok(groups
            .into_iter()
            .map(|g| UsageStats {
                p50_ms: nearest_rank(&g.durations, 50),
                p95_ms: nearest_rank(&g.durations, 95),
                ..g.stats
            })
            .collect())
    }
}

#[derive(Debug, Clone, Default)]
pub struct MissFilter {
    pub kind: Option<MissKind>,
    pub since: Option<i64>,
}

/// A CLI/MCP graph query recorded in `usage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphCall {
    pub usage_id: i64,
    pub ts: i64,
    pub query: Option<String>,
}

impl GraphDb {
    /// # Errors
    ///
    /// Returns [`GraphDbError`] if the insert fails.
    pub fn record_miss(&self, event: &MissEvent) -> Result<()> {
        self.insert_miss(event, None)
    }

    /// Records a `followup` miss tied to the graph call it followed.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDbError`] if the insert fails.
    pub fn record_followup(&self, event: &MissEvent, call: &GraphCall) -> Result<()> {
        self.insert_miss(event, Some(call.usage_id))
    }

    fn insert_miss(&self, event: &MissEvent, usage_id: Option<i64>) -> Result<()> {
        self.conn.busy_timeout(USAGE_BUSY_TIMEOUT)?;
        let inserted = self.conn.execute(
            "INSERT INTO graph_misses (ts, session, kind, query, pattern, reason, usage_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                now_timestamp(),
                event.session,
                event.kind.as_str(),
                event.query,
                event.pattern,
                event.reason,
                usage_id,
            ],
        );
        let _ = self.conn.busy_timeout(DEFAULT_BUSY_TIMEOUT);
        inserted.map(|_| ()).map_err(Into::into)
    }

    /// The most recent CLI/MCP/hook graph query
    /// recorded for `session`, or `None` if it made none. `graph_feedback`
    /// reports on a query rather than making one, so it never counts.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDbError`] if the query fails.
    pub fn last_graph_call(&self, session: &str) -> Result<Option<GraphCall>> {
        self.conn
            .query_row(
                "SELECT id, ts, query FROM usage
                 WHERE session = ?1 AND source IN ('cli', 'mcp', 'hook')
                   AND command != 'graph_feedback'
                 ORDER BY ts DESC, id DESC LIMIT 1",
                params![session],
                |r| {
                    Ok(GraphCall {
                        usage_id: r.get(0)?,
                        ts: r.get(1)?,
                        query: r.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Whether a followup miss has already been recorded for `call`.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDbError`] if the query fails.
    pub fn has_followup_for(&self, call: &GraphCall) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM graph_misses WHERE usage_id = ?1)",
                params![call.usage_id],
                |r| r.get(0),
            )
            .map_err(Into::into)
    }

    /// Recorded misses, newest first, optionally filtered by kind and by a
    /// minimum timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDbError`] if the query fails.
    pub fn list_misses(&self, filter: &MissFilter) -> Result<Vec<MissRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, ts, session, kind, query, pattern, reason FROM graph_misses
             WHERE (?1 IS NULL OR kind = ?1) AND (?2 IS NULL OR ts >= ?2)
             ORDER BY ts DESC, id DESC",
        )?;
        let kind = filter.kind.map(MissKind::as_str);
        let rows = stmt.query_map(params![kind, filter.since], |r| {
            Ok(MissRecord {
                id: r.get(0)?,
                ts: r.get(1)?,
                session: r.get(2)?,
                kind: r.get::<_, String>(3)?.parse().map_err(|err| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(err),
                    )
                })?,
                query: r.get(4)?,
                pattern: r.get(5)?,
                reason: r.get(6)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    /// Deletes `graph_misses` and `usage` rows older than `retention_days`.
    /// Returns the number of `graph_misses` rows removed.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDbError`] if either statement fails.
    pub fn prune(&self, retention_days: i64) -> Result<usize> {
        let cutoff = now_timestamp() - retention_days * 24 * 60 * 60;
        let removed = self
            .conn
            .execute("DELETE FROM graph_misses WHERE ts < ?1", params![cutoff])?;
        self.conn
            .execute("DELETE FROM usage WHERE ts < ?1", params![cutoff])?;
        Ok(removed)
    }
}
