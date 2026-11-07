//! `script_approvals`: the content hashes a human approved for scripts.
//!
//! One row per hall-relative path. Only `ivar script approve` writes it, and
//! the page reindex, the sweep and the drain never touch it. Only the hash is
//! stored, never the script.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, params};

use super::db::open_approvals_read_only;

/// Records `sha256` as the approved content of `path`, replacing any earlier
/// approval of that path.
///
/// # Errors
///
/// When the write fails.
pub fn record(
    conn: &Connection,
    path: &str,
    sha256: &str,
    approved_at: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO script_approvals (path, sha256, approved_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(path) DO UPDATE SET sha256 = excluded.sha256, approved_at = excluded.approved_at",
        params![path, sha256, approved_at],
    )?;
    Ok(())
}

/// Every recorded approval, by path, read without creating or migrating the
/// database. Empty when the file is absent or predates approvals.
///
/// # Errors
///
/// When the file exists but cannot be read.
pub fn read_all(db_path: &Path) -> rusqlite::Result<BTreeMap<String, String>> {
    let Some(conn) = open_approvals_read_only(db_path)? else {
        return Ok(BTreeMap::new());
    };
    let mut stmt = conn.prepare("SELECT path, sha256 FROM script_approvals")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/approvals.rs"]
mod tests;
