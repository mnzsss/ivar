use std::time::Duration;

use rusqlite::params;

use super::GraphDb;
use super::types::{Result, now_timestamp};
use crate::domain::graph::{UsageEvent, UsageSource, UsageStats};

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
    pub fn record_usage(&self, event: &UsageEvent) -> Result<()> {
        // A usage write must never hold a query hostage to another session's lock.
        self.conn.busy_timeout(USAGE_BUSY_TIMEOUT)?;
        let inserted = self.conn.execute(
            "INSERT INTO usage (command, source, ts, duration_ms, result_count, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.command,
                event.source.as_str(),
                now_timestamp(),
                event.duration_ms as i64,
                event.result_count.map(|c| i64::try_from(c).unwrap_or(i64::MAX)),
                event.error,
            ],
        );
        let _ = self.conn.busy_timeout(DEFAULT_BUSY_TIMEOUT);
        inserted.map(|_| ()).map_err(Into::into)
    }

    pub fn usage_summary(&self) -> Result<Vec<UsageStats>> {
        let mut stmt = self.conn.prepare(
            "SELECT command, source, ts, duration_ms, result_count, error
             FROM usage ORDER BY command, source, duration_ms",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(UsageRow {
                command: r.get(0)?,
                source: r.get(1)?,
                ts: r.get(2)?,
                duration_ms: r.get::<_, i64>(3)?.max(0) as u64,
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
