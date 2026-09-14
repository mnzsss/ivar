//! Pure synchronous SQLite database layer for codebase graph storage and querying.

pub mod edges;
pub mod index;
pub mod layer;
pub mod repo;
pub mod symbols;
pub mod types;

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

    /// Opens an in-memory SQLite database initialized with the graph schema.
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
    /// Borrows the underlying SQLite connection.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Mutably borrows the underlying SQLite connection.
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Returns high-level statistics of the indexed codebase graph.
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

        Ok(GraphStats {
            repo_count: repo_count as usize,
            file_count: file_count as usize,
            symbol_count: symbol_count as usize,
            edge_count: edge_count as usize,
            db_size_bytes: (page_count * page_size) as u64,
            layers,
        })
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/store/graph/db.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../../tests/unit/store/graph/views.rs"]
mod views_tests;
