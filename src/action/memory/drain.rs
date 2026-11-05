//! Drains the capture spool into the `observations` table. Each spool file is
//! renamed to `*.draining` before it is read and removed only after its rows
//! commit, so a crash replays the file and `event_id` uniqueness absorbs it.

use std::error::Error;
use std::fs::Metadata;
use std::time::SystemTime;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use super::capture::sanitize_context;
use crate::domain::observation::Observation;
use crate::domain::sanitize::Sanitized;
use crate::domain::session::rfc3339_now;
use crate::error::Failure;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;
use crate::store::memory::observations::insert_batch;

const INSERT_CHUNK: usize = 500;
const STATE_FILE: &str = "_drain.json";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrainState {
    pub last_drain_at: Option<String>,
    pub last_error: Option<String>,
    pub drained_total: u64,
    pub dropped_total: u64,
    pub dropped_last_epoch_secs: Option<u64>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct DrainReport {
    pub files: usize,
    pub inserted: usize,
    pub skipped_lines: usize,
}

type DrainResult<T> = Result<T, Box<dyn Error>>;

/// Drains every spool file, waiting for `memory.lock`.
///
/// # Errors
///
/// When the lock or the database cannot be used; the spool is left in place.
pub fn drain(layout: &Layout) -> Result<DrainReport, Failure> {
    let _lock = fs::lock_exclusive(&layout.memory_lock())?;
    drain_locked(layout, usize::MAX)
}

/// Drains at most one spool file and returns an empty report instead of
/// waiting when another process holds `memory.lock`.
///
/// # Errors
///
/// When the lock or the database cannot be used; the spool is left in place.
pub fn drain_one(layout: &Layout) -> Result<DrainReport, Failure> {
    let Some(_lock) = fs::try_lock_exclusive(&layout.memory_lock())? else {
        return Ok(DrainReport::default());
    };
    drain_locked(layout, 1)
}

/// The caller must hold `memory.lock`; the lock is not re-entrant.
///
/// # Errors
///
/// When the database or the spool cannot be used; the spool is left in place
/// and the error is recorded in `_drain.json`.
pub(crate) fn drain_locked(layout: &Layout, max_files: usize) -> Result<DrainReport, Failure> {
    let dir = layout.memory_spool_dir();
    let mut state = read_state(layout);
    match drain_files(layout, &dir, max_files) {
        Ok(report) => {
            let acknowledgement = begin_acknowledgement(&dir);
            if acknowledgement.count > 0 {
                state.dropped_total += acknowledgement.count;
                state.dropped_last_epoch_secs = Some(epoch_secs_now());
            }
            state.drained_total += u64::try_from(report.inserted).unwrap_or(u64::MAX);
            state.last_drain_at = Some(rfc3339_now());
            state.last_error = None;
            write_state(&dir, &state)
                .map_err(|error| Failure::failed("memory.drain_failed", error.to_string()))?;
            acknowledgement.finish();
            Ok(report)
        }
        Err(error) => {
            state.last_error = Some(error.to_string());
            let _ = write_state(&dir, &state);
            Err(Failure::failed(
                "memory.drain_failed",
                format!("could not drain the memory spool: {error}"),
            ))
        }
    }
}

pub(super) fn epoch_secs_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[must_use]
pub fn read_state(layout: &Layout) -> DrainState {
    std::fs::read_to_string(layout.memory_spool_dir().join(STATE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_state(dir: &Utf8Path, state: &DrainState) -> DrainResult<()> {
    std::fs::create_dir_all(dir)?;
    fs::write_atomic(&dir.join(STATE_FILE), &serde_json::to_vec_pretty(state)?)?;
    Ok(())
}

fn drain_files(layout: &Layout, dir: &Utf8Path, max_files: usize) -> DrainResult<DrainReport> {
    rotate_new_spools(dir);
    let files = draining_files(dir, max_files);
    let mut report = DrainReport::default();
    if files.is_empty() {
        return Ok(report);
    }
    let ctx = sanitize_context(layout);
    let mut db = MemoryDb::open(layout.memory_db().as_std_path())?;
    for file in files {
        let text = String::from_utf8_lossy(&std::fs::read(&file)?).into_owned();
        let mut batch = Vec::new();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let sanitized = serde_json::from_str::<Observation>(line)
                .ok()
                .and_then(|observation| Sanitized::sanitize(observation, &ctx).ok());
            match sanitized {
                Some(observation) => batch.push(observation),
                None => report.skipped_lines += 1,
            }
        }
        for chunk in batch.chunks(INSERT_CHUNK) {
            report.inserted += insert_batch(&mut db, chunk)?;
        }
        std::fs::remove_file(&file)?;
        report.files += 1;
    }
    Ok(report)
}

/// The timestamp keeps a leftover from an earlier failed drain from being
/// overwritten by the rename. A hook that opened the `.jsonl` just before the
/// rename can still append to the renamed inode after it was read; that
/// window is microseconds and accepted.
fn rotate_new_spools(dir: &Utf8Path) {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for path in files_with_extension(dir, "jsonl") {
        let _ = std::fs::rename(&path, path.with_extension(format!("{nanos}.draining")));
    }
}

fn draining_files(dir: &Utf8Path, max_files: usize) -> Vec<Utf8PathBuf> {
    let modified = |path: &Utf8PathBuf| {
        std::fs::metadata(path)
            .and_then(|metadata: Metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    };
    let mut files = files_with_extension(dir, "draining");
    files.sort_by_cached_key(|path| (modified(path), path.clone()));
    files.truncate(max_files);
    files
}

/// Drop counters that were renamed for acknowledgement and counted, but not
/// yet deleted.
struct Acknowledgement {
    count: u64,
    files: Vec<Utf8PathBuf>,
}

impl Acknowledgement {
    fn finish(&self) {
        for file in &self.files {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// Renaming a counter to `*.dropped.<nanos>.ack` before reading it sends any
/// later drop to a fresh `*.dropped`, so none is lost between the read and the
/// delete. The `.ack` files are deleted only after the new total is persisted;
/// one left by a crash is folded into the next drain. A crash between
/// persisting and deleting counts those drops twice, which over-reports rather
/// than hides them.
fn begin_acknowledgement(dir: &Utf8Path) -> Acknowledgement {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for path in files_with_extension(dir, "dropped") {
        let _ = std::fs::rename(&path, path.with_extension(format!("dropped.{nanos}.ack")));
    }
    let files = files_with_extension(dir, "ack");
    let count = files
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .sum();
    Acknowledgement { count, files }
}

fn files_with_extension(dir: &Utf8Path, extension: &str) -> Vec<Utf8PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| Utf8PathBuf::from_path_buf(entry.path()).ok())
        .filter(|path| path.extension() == Some(extension))
        .collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/action/memory/drain.rs"]
mod tests;
