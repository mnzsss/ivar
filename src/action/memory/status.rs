//! `ivar memory status`: where the memory database is and what it carries.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::action::memory::consolidate::{Scope, consolidate_locked, state as consolidate_state};
use crate::action::memory::drain::{drain_locked, epoch_secs_now, read_state};
use crate::action::memory::reindex;
use crate::action::memory::sweep;
use crate::action::{Ctx, discover_hall};
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;
use crate::store::memory::observations::count_by_source_event;
use crate::store::memory::pages::count_by_kind;
use crate::store::memory::retention::{SESSION_RETENTION_DAYS, oldest_session_last_observation};
use crate::store::memory::spool;
use crate::store::memory::usage::{self, USAGE_RETENTION_DAYS};

#[derive(Debug, Clone, Serialize)]
pub struct MemoryStatusOutcome {
    pub path: Utf8PathBuf,
    pub exists: bool,
    pub schema_version: Option<i64>,
    pub application_id: Option<i32>,
    pub size_bytes: Option<u64>,
    pub observations: Vec<ObservationCount>,
    pub spool: SpoolStatus,
    pub last_drain_at: Option<String>,
    pub last_drain_error: Option<String>,
    pub retention: RetentionStatus,
    pub pages: PagesStatus,
    pub consolidation: ConsolidationStatus,
    pub usage: UsageStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageStatus {
    pub window_days: u64,
    #[serde(flatten)]
    pub summary: usage::UsageSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConsolidationStatus {
    pub last_run_at: Option<String>,
    pub last_error: Option<String>,
    pub written_total: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct KindCount {
    pub kind: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PagesStatus {
    pub by_kind: Vec<KindCount>,
    pub last_reindex_at: Option<String>,
    pub last_reindex_error: Option<String>,
    pub refresh_error: Option<String>,
    pub invalid: usize,
    pub anchors_pending: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetentionStatus {
    pub window_days: u64,
    pub last_sweep_at: Option<String>,
    pub last_sweep_error: Option<String>,
    pub swept_total: u64,
    pub oldest_session_observation_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObservationCount {
    pub source_event: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpoolStatus {
    pub files: usize,
    pub bytes: u64,
    pub dropped: u64,
    pub dropped_total: u64,
}

impl SpoolStatus {
    fn read(dir: &Utf8Path) -> Self {
        let pending: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|x| x == "jsonl" || x == "draining")
            })
            .collect();
        let bytes = pending
            .iter()
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();
        Self {
            files: pending.len(),
            bytes,
            dropped: spool::dropped_count(dir),
            dropped_total: 0,
        }
    }
}

/// Moves a pre-split hall first, so the file reported is always the memory
/// database and never the old graph, then opens it for writing, which
/// repairs a memory database an older ivar polluted.
///
/// # Errors
///
/// Outside a hall, while the split is busy, or when the file cannot be read.
pub fn status(ctx: &Ctx) -> Outcome<MemoryStatusOutcome> {
    let layout = discover_hall(ctx)?;
    status_in(&layout)
}

/// # Errors
///
/// While the split is busy, or when the file cannot be read.
pub fn status_in(layout: &Layout) -> Outcome<MemoryStatusOutcome> {
    crate::action::graph::graph_db_path(layout)?;
    let _lock = fs::lock_exclusive(&layout.memory_lock())?;
    let path = layout.memory_db();
    drop(MemoryDb::open(path.as_std_path()).map_err(|error| {
        Failure::failed(
            "memory.open_failed",
            format!("could not open `{path}`: {error}"),
        )
    })?);
    let _ = drain_locked(layout, usize::MAX);
    let size_bytes = fs::stat(&path)?.map(|metadata| metadata.len());
    let inspection = MemoryDb::inspect(path.as_std_path()).map_err(|error| {
        Failure::failed(
            "memory.inspect_failed",
            format!("could not read `{path}`: {error}"),
        )
    })?;
    let mut db = MemoryDb::open(path.as_std_path()).map_err(|error| {
        Failure::failed(
            "memory.open_failed",
            format!("could not open `{path}`: {error}"),
        )
    })?;
    let _ = consolidate_locked(layout, &mut db, epoch_secs_now(), Scope::Idle);
    let (invalid, refresh_error) = match reindex::refresh_locked(layout, &mut db, false) {
        Ok(report) => (report.invalid.len(), None),
        Err(failure) => (0, Some(failure.what)),
    };
    let by_kind = count_by_kind(&db)
        .map_err(|error| Failure::failed("memory.read_failed", error.to_string()))?
        .into_iter()
        .map(|(kind, count)| KindCount { kind, count })
        .collect();
    let mut read_errors = Vec::new();
    let anchors_pending = crate::store::memory::anchors::pending_count(db.conn())
        .map_err(|error| read_errors.push(format!("anchor count: {error}")))
        .unwrap_or(0);
    let reindex_state = reindex::read_state(layout);
    let pages = PagesStatus {
        by_kind,
        last_reindex_at: reindex_state.last_reindex_at,
        last_reindex_error: reindex_state.last_error,
        refresh_error,
        invalid,
        anchors_pending,
    };
    let observations = count_by_source_event(&db)
        .map_err(|error| Failure::failed("memory.read_failed", error.to_string()))?
        .into_iter()
        .map(|(source_event, count)| ObservationCount {
            source_event,
            count,
        })
        .collect();
    let since =
        i64::try_from(epoch_secs_now().saturating_sub(USAGE_RETENTION_DAYS * 86_400)).unwrap_or(0);
    let summary = usage::summary(db.conn(), since)
        .map_err(|error| read_errors.push(format!("usage summary: {error}")))
        .unwrap_or_default();
    let usage = UsageStatus {
        window_days: USAGE_RETENTION_DAYS,
        summary,
        usage_error: (!read_errors.is_empty()).then(|| read_errors.join("; ")),
    };
    let state = read_state(layout);
    let mut spool = SpoolStatus::read(&layout.memory_spool_dir());
    spool.dropped_total = state.dropped_total;
    let sweep_state = sweep::read_state(layout);
    let retention = RetentionStatus {
        window_days: SESSION_RETENTION_DAYS,
        last_sweep_at: sweep_state.last_sweep_at,
        last_sweep_error: sweep_state.last_error,
        swept_total: sweep_state.swept_total,
        oldest_session_observation_at: oldest_session_last_observation(path.as_std_path()),
    };
    let consolidate_state = consolidate_state::read_state(layout);
    let consolidation = ConsolidationStatus {
        last_run_at: consolidate_state.last_run_at,
        last_error: consolidate_state.last_error,
        written_total: consolidate_state.written_total,
    };
    Ok(Report::new(MemoryStatusOutcome {
        path,
        exists: true,
        schema_version: Some(inspection.schema_version),
        application_id: Some(inspection.application_id),
        size_bytes,
        observations,
        spool,
        last_drain_at: state.last_drain_at,
        last_drain_error: state.last_error,
        retention,
        pages,
        consolidation,
        usage,
    }))
}

impl WriteHuman for MemoryStatusOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Memory database: {}", self.path)?;
        let or_unknown = |value: Option<String>| value.unwrap_or_else(|| "unknown".to_owned());
        writeln!(
            w,
            "  schema version: {}",
            or_unknown(self.schema_version.map(|v| v.to_string()))
        )?;
        writeln!(
            w,
            "  application id: {}",
            or_unknown(self.application_id.map(|v| format!("{v:#x}")))
        )?;
        writeln!(
            w,
            "  size:           {} bytes",
            or_unknown(self.size_bytes.map(|v| v.to_string()))
        )?;
        for count in &self.observations {
            writeln!(w, "  {}: {}", count.source_event, count.count)?;
        }
        writeln!(
            w,
            "  spool: {} files, {} bytes, {} dropped",
            self.spool.files,
            self.spool.bytes,
            self.spool.dropped_total + self.spool.dropped
        )?;
        writeln!(
            w,
            "  last drain: {}",
            self.last_drain_at.as_deref().unwrap_or("never")
        )?;
        if let Some(error) = &self.last_drain_error {
            writeln!(w, "  last drain error: {error}")?;
        }
        writeln!(
            w,
            "  retention: {} days, last sweep {}, {} deleted",
            self.retention.window_days,
            self.retention.last_sweep_at.as_deref().unwrap_or("never"),
            self.retention.swept_total
        )?;
        if let Some(error) = &self.retention.last_sweep_error {
            writeln!(w, "  last sweep error: {error}")?;
        }
        let u = &self.usage.summary;
        writeln!(
            w,
            "  queries ({} days): {} cli ({} empty, {:.0}% miss), {} mcp ({} empty, {:.0}% miss); misses: {} followup, {} skipped",
            self.usage.window_days,
            u.queries_cli,
            u.empty_cli,
            u.miss_rate_cli * 100.0,
            u.queries_mcp,
            u.empty_mcp,
            u.miss_rate_mcp * 100.0,
            u.followup,
            u.skipped
        )?;
        if let Some(error) = &self.usage.usage_error {
            writeln!(w, "  usage failed: {error}")?;
        }
        for count in &self.pages.by_kind {
            writeln!(w, "  pages {}: {}", count.kind, count.count)?;
        }
        writeln!(
            w,
            "  last reindex: {}, {} invalid",
            self.pages.last_reindex_at.as_deref().unwrap_or("never"),
            self.pages.invalid
        )?;
        writeln!(w, "  anchors pending: {}", self.pages.anchors_pending)?;
        if let Some(error) = &self.pages.last_reindex_error {
            writeln!(w, "  last reindex error: {error}")?;
        }
        if let Some(error) = &self.pages.refresh_error {
            writeln!(w, "  page refresh failed: {error}")?;
        }
        writeln!(
            w,
            "  consolidation: last {}, {} page(s) written",
            self.consolidation.last_run_at.as_deref().unwrap_or("never"),
            self.consolidation.written_total
        )?;
        if let Some(error) = &self.consolidation.last_error {
            writeln!(w, "  last consolidation error: {error}")?;
        }
        Ok(())
    }
}
