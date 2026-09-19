//! Pure synchronous SQLite database layer for codebase graph storage and querying.

pub mod edges;
pub mod index;
pub mod layer;
pub mod repo;
pub(crate) mod row;
pub mod symbols;
pub mod types;
pub mod usage;

use std::path::Path;

use rusqlite::Connection;

pub use crate::domain::graph::{Edge, EdgeKind, GraphStats, Provenance, Span, Symbol, SymbolKind};
use crate::store::graph::schema;
pub use types::*;

/// Codebase graph database handle wrapping a synchronous SQLite connection.
pub struct GraphDb {
    conn: Connection,
}

impl std::fmt::Debug for GraphDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphDb").finish()
    }
}

impl GraphDb {
    /// Opens or creates an on-disk database at `path`, configuring WAL mode and schema.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDbError`] if parent directories cannot be created, the
    /// connection cannot be opened, or its pragmas/migrations fail.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        // Sessions of every feature share one hall database: wait out another
        // session's migration or index write instead of failing to open.
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        schema::apply_pragmas(&conn, true)?;
        schema::apply_migrations(&conn)?;
        let db = Self { conn };
        db.ensure_views_base_mode()?;
        Ok(db)
    }

    /// Opens an existing, fully migrated database for a best-effort usage write.
    /// Never creates the file, switches journal mode, or runs migrations.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDbError`] if the connection cannot be opened or its
    /// schema version cannot be read, or the message variant if the schema
    /// is not migrated.
    pub fn open_for_usage(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(usage::USAGE_BUSY_TIMEOUT)?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < schema::SCHEMA_VERSION {
            return Err(GraphDbError::Message(format!(
                "graph database at schema version {version} is not migrated"
            )));
        }
        Ok(Self { conn })
    }

    /// Opens an in-memory SQLite database initialized with the graph schema.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDbError`] if the connection cannot be opened, or its
    /// pragmas or migrations fail.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::apply_pragmas(&conn, false)?;
        schema::apply_migrations(&conn)?;
        let db = Self { conn };
        db.ensure_views_base_mode()?;
        Ok(db)
    }
    /// Opens an existing database in read-only mode.
    /// Does not create directories, does not mutate journal mode, and does not apply migrations.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDbError`] if the connection cannot be opened or the read-only pragmas cannot be set.
    pub fn open_read_only(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let db = Self { conn };
        let _ = db.ensure_views_base_mode();
        db.conn.execute_batch(
            "PRAGMA query_only = ON;
             PRAGMA foreign_keys = ON;",
        )?;
        Ok(db)
    }
    /// Runs `f` inside a `BEGIN IMMEDIATE` transaction, committing on success and
    /// rolling back on error.
    fn in_transaction<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        match f() {
            Ok(value) => {
                self.conn.execute_batch("COMMIT;")?;
                Ok(value)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }

    /// Borrows the underlying SQLite connection.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Mutably borrows the underlying SQLite connection.
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Returns high-level statistics of the indexed codebase graph.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDbError`] if any of the count queries fails.
    pub fn stats(&self) -> Result<GraphStats> {
        let repo_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM repos WHERE id NOT LIKE '%/%'",
            [],
            |r| r.get(0),
        )?;
        let file_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM files WHERE repo NOT LIKE '%/%'",
            [],
            |r| r.get(0),
        )?;
        let symbol_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM symbols WHERE repo NOT LIKE '%/%'",
            [],
            |r| r.get(0),
        )?;
        let edge_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM edges WHERE repo NOT LIKE '%/%'",
            [],
            |r| r.get(0),
        )?;
        let page_count: i64 = self
            .conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .unwrap_or(0);
        let page_size: i64 = self
            .conn
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .unwrap_or(4096);
        let layers = self.get_all_layer_stats().unwrap_or_default();
        let usage = self.usage_summary().unwrap_or_default();

        Ok(GraphStats {
            repo_count: usize::try_from(repo_count).unwrap_or(usize::MAX),
            file_count: usize::try_from(file_count).unwrap_or(usize::MAX),
            symbol_count: usize::try_from(symbol_count).unwrap_or(usize::MAX),
            edge_count: usize::try_from(edge_count).unwrap_or(usize::MAX),
            db_size_bytes: u64::try_from(page_count * page_size).unwrap_or(u64::MAX),
            layers,
            usage,
        })
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/store/graph/db.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../../tests/unit/store/graph/views.rs"]
mod views_tests;
