//! Watch scopes CRUD and settlement queries for graph watcher.

use rusqlite::{OptionalExtension, params};

use super::GraphDb;
use super::types::{Result, now_timestamp};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchScopeRow {
    pub scope: String,
    pub observed: i64,
    pub indexed: i64,
    pub needs_catchup: bool,
    pub error: Option<String>,
    pub updated_at: i64,
}

impl GraphDb {
    /// Upserts a watch scope row, setting `needs_catchup = 1`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::store::graph::db::GraphDbError`] if the statement fails.
    pub fn watch_register(&self, scope: &str) -> Result<()> {
        let ts = now_timestamp();
        self.conn
            .prepare_cached(
                "INSERT INTO watch_scopes (scope, updated_at) VALUES (?1, ?2)
                 ON CONFLICT(scope) DO UPDATE SET needs_catchup = 1, updated_at = excluded.updated_at",
            )?
            .execute(params![scope, ts])?;
        Ok(())
    }

    /// Increments observed sequence for a scope and returns the new observed value.
    ///
    /// # Errors
    ///
    /// Returns [`crate::store::graph::db::GraphDbError`] if the statement fails.
    pub fn watch_bump_observed(&self, scope: &str) -> Result<i64> {
        let ts = now_timestamp();
        let observed = self
            .conn
            .prepare_cached(
                "UPDATE watch_scopes SET observed = observed + 1, updated_at = ?2 WHERE scope = ?1 RETURNING observed",
            )?
            .query_row(params![scope, ts], |row| row.get(0))?;
        Ok(observed)
    }

    /// Marks a scope finished up to `seq`, optionally clearing the `needs_catchup` flag.
    ///
    /// # Errors
    ///
    /// Returns [`crate::store::graph::db::GraphDbError`] if the statement fails.
    pub fn watch_finish(&self, scope: &str, seq: i64, clear_catchup: bool) -> Result<()> {
        let ts = now_timestamp();
        self.conn
            .prepare_cached(
                "UPDATE watch_scopes SET indexed = MAX(indexed, ?2), error = NULL,
                     needs_catchup = CASE WHEN ?3 THEN 0 ELSE needs_catchup END, updated_at = ?4
                 WHERE scope = ?1",
            )?
            .execute(params![scope, seq, clear_catchup, ts])?;
        Ok(())
    }

    /// Records an indexing error for a scope and marks it for catchup.
    ///
    /// # Errors
    ///
    /// Returns [`crate::store::graph::db::GraphDbError`] if the statement fails.
    pub fn watch_fail(&self, scope: &str, error: &str) -> Result<()> {
        let ts = now_timestamp();
        self.conn
            .prepare_cached(
                "UPDATE watch_scopes SET error = ?2, needs_catchup = 1, updated_at = ?3 WHERE scope = ?1",
            )?
            .execute(params![scope, error, ts])?;
        Ok(())
    }

    /// Flags a scope as needing catch-up.
    ///
    /// # Errors
    ///
    /// Returns [`crate::store::graph::db::GraphDbError`] if the statement fails.
    pub fn watch_flag_catchup(&self, scope: &str) -> Result<()> {
        let ts = now_timestamp();
        self.conn
            .prepare_cached(
                "UPDATE watch_scopes SET needs_catchup = 1, updated_at = ?2 WHERE scope = ?1",
            )?
            .execute(params![scope, ts])?;
        Ok(())
    }

    /// Marks all registered watch scopes as needing catch-up.
    ///
    /// # Errors
    ///
    /// Returns [`crate::store::graph::db::GraphDbError`] if the statement fails.
    pub fn watch_mark_all_catchup(&self) -> Result<()> {
        self.conn
            .prepare_cached("UPDATE watch_scopes SET needs_catchup = 1")?
            .execute([])?;
        Ok(())
    }

    /// Deletes a watch scope row.
    ///
    /// # Errors
    ///
    /// Returns [`crate::store::graph::db::GraphDbError`] if the statement fails.
    pub fn watch_forget(&self, scope: &str) -> Result<()> {
        self.conn
            .prepare_cached("DELETE FROM watch_scopes WHERE scope = ?1")?
            .execute(params![scope])?;
        Ok(())
    }

    /// Checks if all given scopes are registered, up to date with observed bursts, without pending catchup or error.
    ///
    /// # Errors
    ///
    /// Returns [`crate::store::graph::db::GraphDbError`] if the query fails.
    pub fn watch_settled(&self, scopes: &[&str]) -> Result<bool> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT indexed >= observed AND needs_catchup = 0 AND error IS NULL FROM watch_scopes WHERE scope = ?1",
        )?;
        for scope in scopes {
            let settled: Option<bool> = stmt.query_row(params![scope], |row| row.get(0)).optional()?;
            if !settled.unwrap_or(false) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Returns all registered watch scope rows ordered by scope.
    ///
    /// # Errors
    ///
    /// Returns [`crate::store::graph::db::GraphDbError`] if the query fails.
    pub fn watch_scopes(&self) -> Result<Vec<WatchScopeRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT scope, observed, indexed, needs_catchup, error, updated_at FROM watch_scopes ORDER BY scope",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(WatchScopeRow {
                    scope: row.get(0)?,
                    observed: row.get(1)?,
                    indexed: row.get(2)?,
                    needs_catchup: row.get::<_, i64>(3)? != 0,
                    error: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/store/graph/watch.rs"]
mod tests;
