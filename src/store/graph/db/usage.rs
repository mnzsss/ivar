use std::time::Duration;

use rusqlite::params;

use super::GraphDb;
use super::types::{Result, now_timestamp};
use crate::domain::graph::{UsageEvent, UsageSource, UsageStats};

const USAGE_BUSY_TIMEOUT: Duration = Duration::from_millis(50);
const DEFAULT_BUSY_TIMEOUT: Duration = Duration::from_secs(10);

fn source_str(source: UsageSource) -> &'static str {
    match source {
        UsageSource::Cli => "cli",
        UsageSource::Mcp => "mcp",
    }
}

fn nearest_rank(sorted: &[u64], pct: u64) -> u64 {
    let rank = (sorted.len() as u64 * pct).div_ceil(100).max(1) as usize;
    sorted.get(rank - 1).copied().unwrap_or(0)
}

impl GraphDb {
    pub fn record_usage(&self, event: &UsageEvent) -> Result<()> {
        // A usage write must never hold a query hostage to another session's lock.
        self.conn.busy_timeout(USAGE_BUSY_TIMEOUT)?;
        let inserted = self.conn.execute(
            "INSERT INTO usage (command, source, ts, duration_ms, result_count, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.command,
                source_str(event.source),
                now_timestamp(),
                event.duration_ms as i64,
                event.result_count.map(|c| c as i64),
                event.error,
            ],
        );
        self.conn.busy_timeout(DEFAULT_BUSY_TIMEOUT)?;
        inserted.map(|_| ()).map_err(Into::into)
    }

    pub fn usage_summary(&self) -> Result<Vec<UsageStats>> {
        let mut groups = self.conn.prepare(
            "SELECT command, source, COUNT(*), MAX(ts), COALESCE(SUM(result_count = 0), 0), SUM(error)
             FROM usage GROUP BY command, source ORDER BY command, source",
        )?;
        let rows = groups
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut durations = self.conn.prepare(
            "SELECT duration_ms FROM usage WHERE command = ?1 AND source = ?2 ORDER BY duration_ms",
        )?;
        rows.into_iter()
            .map(|(command, source, count, last_used, empty, errors)| {
                let sorted = durations
                    .query_map(params![command, source], |r| r.get::<_, i64>(0))?
                    .map(|d| d.map(|d| d as u64))
                    .collect::<rusqlite::Result<Vec<u64>>>()?;
                Ok(UsageStats {
                    command,
                    source: if source == "mcp" {
                        UsageSource::Mcp
                    } else {
                        UsageSource::Cli
                    },
                    count: count as u64,
                    last_used,
                    empty_count: empty as u64,
                    error_count: errors as u64,
                    p50_ms: nearest_rank(&sorted, 50),
                    p95_ms: nearest_rank(&sorted, 95),
                })
            })
            .collect()
    }
}
