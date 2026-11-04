//! Deletes `session.*` observations whose session ended more than the
//! retention window ago. Runs under a non-waiting `memory.lock` and never
//! creates `memory.db`.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::domain::session::rfc3339_now;
use crate::error::Failure;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;
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
    pub busy: bool,
}

/// Sweeps expired sessions, or reports `busy` instead of waiting when another
/// process holds `memory.lock`.
///
/// # Errors
///
/// When the lock or the database cannot be used; the error is recorded in
/// `_sweep.json`.
pub fn sweep(layout: &Layout, now_epoch_secs: u64) -> Result<SweepReport, Failure> {
    let Some(_lock) = fs::try_lock_exclusive(&layout.memory_lock())? else {
        return Ok(SweepReport {
            deleted: 0,
            busy: true,
        });
    };
    let cutoff = cutoff_before(now_epoch_secs, SESSION_RETENTION_DAYS);
    let mut state = read_state(layout);
    let outcome = delete_all_expired(layout, &cutoff);
    match &outcome {
        Ok(deleted) => {
            state.swept_total += u64::try_from(*deleted).unwrap_or(u64::MAX);
            state.last_sweep_at = Some(rfc3339_now());
            state.last_error = None;
        }
        Err(error) => state.last_error = Some(error.clone()),
    }
    let _ = write_state(layout, &state);
    outcome
        .map(|deleted| SweepReport {
            deleted,
            busy: false,
        })
        .map_err(|error| Failure::failed("memory.sweep_failed", error))
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

#[cfg(test)]
#[path = "../../../tests/unit/action/memory/sweep.rs"]
mod tests;
