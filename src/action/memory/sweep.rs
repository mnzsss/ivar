//! Deletes `session.*` observations whose session ended more than the
//! retention window ago. Runs under a non-waiting `memory.lock` held for the
//! whole sweep and never creates `memory.db`.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use camino::Utf8Path;

use crate::action::memory::consolidate;
use crate::action::memory::page_file::SESSION_PAGES_DIR;
use crate::action::memory::reindex;
use crate::domain::session::{epoch_secs_from_rfc3339, rfc3339_now};
use crate::error::Failure;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;
use crate::store::memory::page_source::parse_page;
use crate::store::memory::retention::{
    SESSION_RETENTION_DAYS, cutoff_before, delete_expired_sessions,
};

const BATCH: usize = 1000;
const STATE_FILE: &str = "_sweep.json";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SweepState {
    pub last_sweep_at: Option<String>,
    pub last_error: Option<String>,
    pub swept_total: u64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub deleted: usize,
    pub pages_removed: usize,
    pub busy: bool,
}

/// Sweeps expired sessions, or reports `busy` instead of waiting when another
/// process holds `memory.lock`. The lock is held for the whole sweep.
///
/// # Errors
///
/// When the lock or the database cannot be used; the error is recorded in
/// `_sweep.json`.
pub fn sweep(layout: &Layout, now_epoch_secs: u64) -> Result<SweepReport, Failure> {
    let mut state = read_state(layout);
    // Held across every batch: releasing between batches lets a sibling
    // process holding a duplicate of the closed fd make the next attempt busy.
    let _lock = match fs::try_lock_exclusive(&layout.memory_lock()) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            return Ok(SweepReport {
                busy: true,
                ..SweepReport::default()
            });
        }
        Err(error) => return Err(record_failure(layout, &mut state, error.to_string())),
    };
    let cutoff = cutoff_before(now_epoch_secs, SESSION_RETENTION_DAYS);
    let deleted = delete_all_expired(layout, &cutoff)
        .map_err(|error| record_failure(layout, &mut state, error))?;
    let pages_removed = expire_session_pages(layout, now_epoch_secs)
        .map_err(|error| record_failure(layout, &mut state, error))?;
    state.swept_total += u64::try_from(deleted).unwrap_or(u64::MAX);
    state.last_sweep_at = Some(rfc3339_now());
    state.last_error = None;
    let _ = write_state(layout, &state);
    Ok(SweepReport {
        deleted,
        pages_removed,
        busy: false,
    })
}

fn record_failure(layout: &Layout, state: &mut SweepState, error: String) -> Failure {
    state.last_error = Some(error.clone());
    let _ = write_state(layout, state);
    Failure::failed("memory.sweep_failed", error)
}

/// # Errors
///
/// As [`sweep`].
pub fn sweep_now(layout: &Layout) -> Result<SweepReport, Failure> {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    sweep(layout, secs)
}

#[must_use]
pub fn read_state(layout: &Layout) -> SweepState {
    std::fs::read_to_string(layout.memory_spool_dir().join(STATE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_state(layout: &Layout, state: &SweepState) -> Result<(), Box<dyn std::error::Error>> {
    let dir = layout.memory_spool_dir();
    std::fs::create_dir_all(&dir)?;
    fs::write_atomic(&dir.join(STATE_FILE), &serde_json::to_vec_pretty(state)?)?;
    Ok(())
}

fn delete_all_expired(layout: &Layout, cutoff: &str) -> Result<usize, String> {
    if !layout.memory_db().exists() {
        return Ok(0);
    }
    let mut db = MemoryDb::open(layout.memory_db().as_std_path()).map_err(|e| e.to_string())?;
    let mut total = 0;
    loop {
        let deleted = delete_expired_sessions(&mut db, cutoff, BATCH).map_err(|e| e.to_string())?;
        total += deleted;
        if deleted < BATCH {
            break;
        }
    }
    if total > 0 {
        db.conn()
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .map_err(|e| e.to_string())?;
    }
    Ok(total)
}

/// A page ages from its conversation's marker in `_consolidate.json`, keyed by
/// the file stem, or from its mtime when the marker is gone. A page whose
/// frontmatter does not parse is left to lint and doctor.
fn expire_session_pages(layout: &Layout, now_epoch_secs: u64) -> Result<usize, String> {
    let dir = layout.root().join(SESSION_PAGES_DIR);
    let Ok(entries) = dir.read_dir_utf8() else {
        return Ok(0);
    };
    let state = consolidate::state::read_state(layout);
    let now = i64::try_from(now_epoch_secs).unwrap_or(i64::MAX);
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension() != Some("md") {
            continue;
        }
        let Some(name) = path.file_stem().map(str::to_owned) else {
            continue;
        };
        let rel = format!("{SESSION_PAGES_DIR}{name}.md");
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(page) = parse_page(&rel, &text) else {
            continue;
        };
        let last_at = state
            .markers
            .get(&name)
            .and_then(|marker| epoch_secs_from_rfc3339(&marker.last_at))
            .or_else(|| mtime_epoch(path))
            .unwrap_or(now);
        if consolidate::is_expired(&page.meta, last_at, now) {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
            // The marker stays so consolidation does not recreate the page
            // without its expires_at; consolidation prunes it after PAGE_TTL_DAYS.
            removed += 1;
        }
    }
    if removed > 0 && layout.memory_db().exists() {
        let mut db = MemoryDb::open(layout.memory_db().as_std_path()).map_err(|e| e.to_string())?;
        reindex::refresh_locked(layout, &mut db, false).map_err(|f| f.what)?;
    }
    Ok(removed)
}

fn mtime_epoch(path: &Utf8Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    i64::try_from(
        modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_secs(),
    )
    .ok()
}

#[cfg(test)]
#[path = "../../../tests/unit/action/memory/sweep.rs"]
mod tests;
