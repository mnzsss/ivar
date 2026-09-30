//! Persistent update-check cache on the local filesystem.

use std::time::{SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::infra::fs;

/// The persisted update-check payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Unix timestamp (seconds) when the check was initiated.
    pub last_checked_at: u64,
    /// The latest published version tag/string, if known.
    pub latest_version: Option<String>,
}

/// The cache file location: `<cache_dir>/ivar/update-check.json`.
pub fn cache_path() -> Option<Utf8PathBuf> {
    fs::cache_dir()
        .ok()
        .map(|base| base.join("ivar").join("update-check.json"))
}

/// Read the cache entry, or `None` if missing or corrupted.
pub fn read(path: &Utf8Path) -> Option<CacheEntry> {
    let text = fs::read_text(path).ok()??;
    serde_json::from_str(&text).ok()
}

/// Write the cache entry atomically to `path`.
pub fn write(path: &Utf8Path, entry: &CacheEntry) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Ok(bytes) = serde_json::to_vec(entry) else {
        return false;
    };
    fs::ensure_dir(parent).is_ok() && fs::write_atomic(path, &bytes).is_ok()
}

/// Current time in Unix seconds. Returns `0` on pre-epoch clocks.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
#[path = "../../../tests/unit/action/upgrade/cache.rs"]
mod tests;
