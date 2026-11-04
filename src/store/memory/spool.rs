//! The capture spool: one append-only JSONL file per session under
//! `.ivar/spool/memory/`, written by the capture hook without taking
//! `memory.lock` so an agent never waits behind a drain.

use std::fs::OpenOptions;
use std::io::{self, Write};

use camino::Utf8Path;

use crate::domain::observation::Observation;
use crate::domain::sanitize::Sanitized;

pub const SPOOL_CAP_BYTES: u64 = 8 * 1024 * 1024;

const MAX_KEY_LEN: usize = 128;
const HASHED_KEY_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpoolKey(String);

impl SpoolKey {
    #[must_use]
    pub fn new(ivar_session: Option<&str>, native_session: &str) -> Self {
        let raw = ivar_session.unwrap_or(native_session);
        let safe = !raw.is_empty()
            && raw.len() <= MAX_KEY_LEN
            && !raw.contains("..")
            && raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        let name = if safe {
            raw.to_owned()
        } else {
            crate::infra::hash::text(raw)[..HASHED_KEY_LEN].to_owned()
        };
        Self(name)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Appended {
    Written,
    Dropped,
}

/// Appends the observation as one JSON line to the session's spool file, or
/// counts it as dropped when the file would pass [`SPOOL_CAP_BYTES`]. Only a
/// sanitized observation can reach the disk.
///
/// # Errors
///
/// Returns the I/O error from serialising, creating the directory or writing
/// the file.
pub fn append(
    dir: &Utf8Path,
    key: &SpoolKey,
    observation: &Sanitized<Observation>,
) -> io::Result<Appended> {
    let line = serde_json::to_string(observation.as_observation())?;
    append_line(dir, key, &line)
}

fn append_line(dir: &Utf8Path, key: &SpoolKey, line: &str) -> io::Result<Appended> {
    std::fs::create_dir_all(dir)?;
    let spool = dir.join(format!("{}.jsonl", key.as_str()));
    let current = session_bytes(dir, key);
    let added = u64::try_from(line.len())
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    // ponytail: the cap is checked without a lock, so two hooks racing at the
    // boundary may exceed it by one line; add a lock if a hard cap matters.
    if current.saturating_add(added) > SPOOL_CAP_BYTES {
        let mut counter = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(format!("{}.dropped", key.as_str())))?;
        counter.write_all(b"x")?;
        return Ok(Appended::Dropped);
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&spool)?;
    file.write_all(format!("{line}\n").as_bytes())?;
    Ok(Appended::Written)
}

/// Bytes of the session's live file plus the `<key>.<nanos>.draining` files a
/// rotation left behind, so the cap holds for a session that is not drained.
fn session_bytes(dir: &Utf8Path, key: &SpoolKey) -> u64 {
    let live = format!("{}.jsonl", key.as_str());
    let rotated_prefix = format!("{}.", key.as_str());
    let is_rotated = |name: &str| {
        name.strip_prefix(&rotated_prefix)
            .and_then(|rest| rest.strip_suffix(".draining"))
            .is_some_and(|nanos| !nanos.is_empty() && nanos.bytes().all(|b| b.is_ascii_digit()))
    };
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name == live || is_rotated(name))
        })
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum()
}

/// Lines dropped at the cap, one byte per drop across every session counter,
/// including counters a drain renamed to `*.ack` but has not recorded yet.
#[must_use]
pub fn dropped_count(dir: &Utf8Path) -> u64 {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext == "dropped" || ext == "ack")
        })
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum()
}

/// Whether any `*.jsonl` or `*.draining` file is waiting to be drained.
#[must_use]
pub fn has_pending(dir: &Utf8Path) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext == "jsonl" || ext == "draining")
        })
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/spool.rs"]
mod tests;
