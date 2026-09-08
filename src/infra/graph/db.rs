//! Pure synchronous SQLite database layer for codebase graph storage and querying.

use std::borrow::Cow;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

use crate::domain::graph::{Edge, EdgeKind, GraphStats, Provenance, Span, Symbol, SymbolKind};
use crate::infra::graph::schema;

/// Error type for database operations.
#[derive(Debug, thiserror::Error)]
pub enum GraphDbError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Message(String),
}

pub type Result<T, E = GraphDbError> = std::result::Result<T, E>;

/// A record representing a repository in the `repos` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRow {
    pub id: String,
    pub root_path: String,
    pub default_branch: String,
    pub last_indexed_commit: Option<String>,
    pub indexed_at: i64,
}

/// A record representing an indexed file in the `files` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub id: i64,
    pub repo: String,
    pub path: String,
    pub content_hash: String,
    pub mtime_ns: i64,
    pub size_bytes: i64,
    pub indexed_at: i64,
}

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
        schema::apply_pragmas(&conn, true)?;
        schema::apply_migrations(&conn)?;
        Ok(Self { conn })
    }

    /// Opens an in-memory SQLite database initialized with the graph schema.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::apply_pragmas(&conn, false)?;
        schema::apply_migrations(&conn)?;
        Ok(Self { conn })
    }

    /// Borrows the underlying SQLite connection.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Mutably borrows the underlying SQLite connection.
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Inserts or updates repository metadata.
    pub fn insert_repo(
        &self,
        id: &str,
        root_path: &str,
        default_branch: &str,
        commit: Option<&str>,
    ) -> Result<()> {
        let now = now_timestamp();
        self.conn.execute(
            "INSERT INTO repos (id, root_path, default_branch, last_indexed_commit, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                root_path = excluded.root_path,
                default_branch = excluded.default_branch,
                last_indexed_commit = excluded.last_indexed_commit,
                indexed_at = excluded.indexed_at",
            params![id, root_path, default_branch, commit, now],
        )?;
        Ok(())
    }

    /// Fetches repository metadata by repository ID.
    pub fn get_repo(&self, id: &str) -> Result<Option<RepoRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, root_path, default_branch, last_indexed_commit, indexed_at FROM repos WHERE id = ?1",
        )?;
        let result = stmt
            .query_row(params![id], |row| {
                Ok(RepoRow {
                    id: row.get(0)?,
                    root_path: row.get(1)?,
                    default_branch: row.get(2)?,
                    last_indexed_commit: row.get(3)?,
                    indexed_at: row.get(4)?,
                })
            })
            .optional()?;
        Ok(result)
    }

    /// Updates the last indexed commit and timestamp for a repository.
    pub fn update_repo_commit(&self, id: &str, commit: &str) -> Result<()> {
        let now = now_timestamp();
        self.conn.execute(
            "UPDATE repos SET last_indexed_commit = ?1, indexed_at = ?2 WHERE id = ?3",
            params![commit, now, id],
        )?;
        Ok(())
    }

    /// Inserts or updates file metadata and returns the row ID.
    pub fn upsert_file(
        &self,
        repo: &str,
        path: &str,
        hash: &str,
        mtime_ns: i64,
        size_bytes: i64,
    ) -> Result<i64> {
        let now = now_timestamp();
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO files (repo, path, content_hash, mtime_ns, size_bytes, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(repo, path) DO UPDATE SET
                content_hash = excluded.content_hash,
                mtime_ns = excluded.mtime_ns,
                size_bytes = excluded.size_bytes,
                indexed_at = excluded.indexed_at
             RETURNING id",
        )?;
        let id: i64 = stmt.query_row(
            params![repo, path, hash, mtime_ns, size_bytes, now],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    /// Fetches file metadata by repo and path.
    pub fn get_file(&self, repo: &str, path: &str) -> Result<Option<FileRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, repo, path, content_hash, mtime_ns, size_bytes, indexed_at FROM files WHERE repo = ?1 AND path = ?2",
        )?;
        let result = stmt
            .query_row(params![repo, path], |row| {
                Ok(FileRow {
                    id: row.get(0)?,
                    repo: row.get(1)?,
                    path: row.get(2)?,
                    content_hash: row.get(3)?,
                    mtime_ns: row.get(4)?,
                    size_bytes: row.get(5)?,
                    indexed_at: row.get(6)?,
                })
            })
            .optional()?;
        Ok(result)
    }

    /// Deletes a file record by repo and relative path (cascades to symbols and edges).
    pub fn delete_file(&self, repo: &str, path: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM files WHERE repo = ?1 AND path = ?2",
            params![repo, path],
        )?;
        Ok(())
    }

    /// Deletes all symbols belonging to a specific file ID (cascades to outbound edges).
    pub fn delete_symbols_for_file(&self, file_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM symbols WHERE file_id = ?1",
            params![file_id],
        )?;
        Ok(())
    }

    /// Bulk inserts symbols in a single transaction and returns their generated IDs.
    pub fn insert_symbols(&self, symbols: &[Symbol]) -> Result<Vec<i64>> {
        if symbols.is_empty() {
            return Ok(Vec::new());
        }
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let res = (|| -> Result<Vec<i64>> {
            let mut stmt = self.conn.prepare_cached(
                "INSERT INTO symbols (file_id, repo, name, kind, scope, signature, docstring, start_line, start_col, end_line, end_col, is_exported)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 RETURNING id",
            )?;
            let mut ids = Vec::with_capacity(symbols.len());
            for sym in symbols {
                let file_id = sym.file_id.ok_or_else(|| {
                    GraphDbError::Message("symbol missing required file_id".to_string())
                })?;
                let kind_str = symbol_kind_to_str(&sym.kind);
                let is_exported = if sym.is_exported { 1 } else { 0 };
                let id: i64 = stmt.query_row(
                    params![
                        file_id,
                        &sym.repo,
                        &sym.name,
                        kind_str.as_ref(),
                        &sym.scope,
                        &sym.signature,
                        &sym.docstring,
                        sym.span.start_line as i64,
                        sym.span.start_col as i64,
                        sym.span.end_line as i64,
                        sym.span.end_col as i64,
                        is_exported,
                    ],
                    |row| row.get(0),
                )?;
                ids.push(id);
            }
            Ok(ids)
        })();

        match res {
            Ok(ids) => {
                self.conn.execute_batch("COMMIT;")?;
                Ok(ids)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }

    /// Bulk inserts edges in a single transaction and returns their generated IDs.
    pub fn insert_edges(&self, edges: &[Edge]) -> Result<Vec<i64>> {
        if edges.is_empty() {
            return Ok(Vec::new());
        }
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let res = (|| -> Result<Vec<i64>> {
            let mut stmt = self.conn.prepare_cached(
                "INSERT INTO edges (repo, file_id, from_symbol_id, to_symbol_id, to_name, kind, provenance, line, col, confidence)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 RETURNING id",
            )?;
            let mut ids = Vec::with_capacity(edges.len());
            for edge in edges {
                let file_id = edge.file_id.ok_or_else(|| {
                    GraphDbError::Message("edge missing required file_id".to_string())
                })?;
                let kind_str = edge_kind_to_str(&edge.kind);
                let prov_str = provenance_to_str(&edge.provenance);
                let id: i64 = stmt.query_row(
                    params![
                        &edge.repo,
                        file_id,
                        edge.from_symbol_id,
                        edge.to_symbol_id,
                        &edge.to_name,
                        kind_str.as_ref(),
                        prov_str,
                        edge.line as i64,
                        edge.col as i64,
                        edge.confidence,
                    ],
                    |row| row.get(0),
                )?;
                ids.push(id);
            }
            Ok(ids)
        })();

        match res {
            Ok(ids) => {
                self.conn.execute_batch("COMMIT;")?;
                Ok(ids)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }

    /// Full-text search across symbols via SQLite FTS5.
    pub fn search_symbols_fts(&self, query: &str, limit: usize) -> Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                    s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported
             FROM symbols_fts fts
             JOIN symbols s ON fts.rowid = s.id
             WHERE symbols_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![query, limit as i64], |row| {
            let id: i64 = row.get(0)?;
            let file_id: i64 = row.get(1)?;
            let repo: String = row.get(2)?;
            let name: String = row.get(3)?;
            let kind_raw: String = row.get(4)?;
            let scope: Option<String> = row.get(5)?;
            let signature: Option<String> = row.get(6)?;
            let docstring: Option<String> = row.get(7)?;
            let start_line: i64 = row.get(8)?;
            let start_col: i64 = row.get(9)?;
            let end_line: i64 = row.get(10)?;
            let end_col: i64 = row.get(11)?;
            let is_exported: i64 = row.get(12)?;

            Ok(Symbol {
                id: Some(id),
                file_id: Some(file_id),
                repo,
                name,
                kind: parse_symbol_kind(&kind_raw),
                scope,
                signature,
                docstring,
                span: Span::new(
                    start_line as usize,
                    start_col as usize,
                    end_line as usize,
                    end_col as usize,
                ),
                is_exported: is_exported != 0,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Re-links dangling edges where `to_symbol_id` is null by matching `to_name` with symbols in the same repository.
    pub fn relink_dangling_edges(&self, repo: &str) -> Result<usize> {
        let count = self.conn.execute(
            "UPDATE edges
             SET to_symbol_id = (
                 SELECT s.id FROM symbols s
                 WHERE s.repo = edges.repo
                   AND s.name = edges.to_name
                 ORDER BY s.is_exported DESC, s.id ASC
                 LIMIT 1
             )
             WHERE repo = ?1
               AND to_symbol_id IS NULL
               AND to_name IS NOT NULL
               AND EXISTS (
                 SELECT 1 FROM symbols s
                 WHERE s.repo = edges.repo
                   AND s.name = edges.to_name
               )",
            params![repo],
        )?;
        Ok(count)
    }

    /// Returns high-level statistics of the indexed codebase graph.
    pub fn stats(&self) -> Result<GraphStats> {
        let repo_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM repos", [], |r| r.get(0))?;
        let file_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        let symbol_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))?;
        let edge_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
        let page_count: i64 = self
            .conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .unwrap_or(0);
        let page_size: i64 = self
            .conn
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .unwrap_or(4096);

        Ok(GraphStats {
            repo_count: repo_count as usize,
            file_count: file_count as usize,
            symbol_count: symbol_count as usize,
            edge_count: edge_count as usize,
            db_size_bytes: (page_count * page_size) as u64,
        })
    }
}

fn now_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn symbol_kind_to_str<'a>(kind: &'a SymbolKind) -> Cow<'a, str> {
    match kind {
        SymbolKind::Fn => "fn".into(),
        SymbolKind::Method => "method".into(),
        SymbolKind::Struct => "struct".into(),
        SymbolKind::Class => "class".into(),
        SymbolKind::Trait => "trait".into(),
        SymbolKind::Interface => "interface".into(),
        SymbolKind::Enum => "enum".into(),
        SymbolKind::Mod => "mod".into(),
        SymbolKind::Const => "const".into(),
        SymbolKind::Other(s) => s.as_str().into(),
    }
}

fn parse_symbol_kind(s: &str) -> SymbolKind {
    match s.to_ascii_lowercase().as_str() {
        "fn" => SymbolKind::Fn,
        "method" => SymbolKind::Method,
        "struct" => SymbolKind::Struct,
        "class" => SymbolKind::Class,
        "trait" => SymbolKind::Trait,
        "interface" => SymbolKind::Interface,
        "enum" => SymbolKind::Enum,
        "mod" => SymbolKind::Mod,
        "const" => SymbolKind::Const,
        other => SymbolKind::Other(other.to_string()),
    }
}

fn edge_kind_to_str<'a>(kind: &'a EdgeKind) -> Cow<'a, str> {
    match kind {
        EdgeKind::Calls => "CALLS".into(),
        EdgeKind::Imports => "IMPORTS".into(),
        EdgeKind::Implements => "IMPLEMENTS".into(),
        EdgeKind::CrossImports => "CROSS_IMPORTS".into(),
        EdgeKind::CrossExecutes => "CROSS_EXECUTES".into(),
        EdgeKind::CrossCallsHttp => "CROSS_CALLS_HTTP".into(),
        EdgeKind::Other(s) => s.as_str().into(),
    }
}

#[allow(dead_code)]
fn parse_edge_kind(s: &str) -> EdgeKind {
    match s {
        "CALLS" | "calls" => EdgeKind::Calls,
        "IMPORTS" | "imports" => EdgeKind::Imports,
        "IMPLEMENTS" | "implements" => EdgeKind::Implements,
        "CROSS_IMPORTS" | "cross_imports" => EdgeKind::CrossImports,
        "CROSS_EXECUTES" | "cross_executes" => EdgeKind::CrossExecutes,
        "CROSS_CALLS_HTTP" | "cross_calls_http" => EdgeKind::CrossCallsHttp,
        other => EdgeKind::Other(other.to_string()),
    }
}

fn provenance_to_str(p: &Provenance) -> &'static str {
    match p {
        Provenance::Extracted => "EXTRACTED",
        Provenance::Inferred => "INFERRED",
        Provenance::Ambiguous => "AMBIGUOUS",
    }
}

#[allow(dead_code)]
fn parse_provenance(s: &str) -> Provenance {
    match s {
        "EXTRACTED" | "extracted" => Provenance::Extracted,
        "INFERRED" | "inferred" => Provenance::Inferred,
        "AMBIGUOUS" | "ambiguous" => Provenance::Ambiguous,
        _ => Provenance::Extracted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_open_in_memory_and_stats() {
        let db = GraphDb::open_in_memory().expect("failed to open in memory db");
        let stats = db.stats().expect("failed to get stats");
        assert_eq!(stats.repo_count, 0);
        assert_eq!(stats.file_count, 0);
        assert_eq!(stats.symbol_count, 0);
        assert_eq!(stats.edge_count, 0);
    }

    #[test]
    fn test_open_on_disk() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("sub").join("graph.db");
        let db = GraphDb::open(&db_path).expect("failed to open on-disk db");
        let stats = db.stats().expect("stats");
        assert_eq!(stats.repo_count, 0);

        // Verify foreign_keys enabled
        let fk: i64 = db
            .conn()
            .query_row("PRAGMA foreign_keys;", [], |r| r.get(0))
            .expect("pragma fk");
        assert_eq!(fk, 1);
    }

    #[test]
    fn test_repo_and_file_crud() {
        let db = GraphDb::open_in_memory().expect("open");
        db.insert_repo("ivar", "/path/to/ivar", "main", Some("abcdef012345"))
            .expect("insert repo");

        let repo = db.get_repo("ivar").expect("get repo").expect("exists");
        assert_eq!(repo.id, "ivar");
        assert_eq!(repo.root_path, "/path/to/ivar");
        assert_eq!(repo.default_branch, "main");
        assert_eq!(repo.last_indexed_commit.as_deref(), Some("abcdef012345"));

        db.update_repo_commit("ivar", "112233445566")
            .expect("update repo commit");
        let repo_updated = db.get_repo("ivar").expect("get repo").expect("exists");
        assert_eq!(
            repo_updated.last_indexed_commit.as_deref(),
            Some("112233445566")
        );

        let file_id = db
            .upsert_file("ivar", "src/main.rs", "hash_abc", 1000, 2048)
            .expect("upsert file");
        assert!(file_id > 0);

        let file = db
            .get_file("ivar", "src/main.rs")
            .expect("get file")
            .expect("exists");
        assert_eq!(file.id, file_id);
        assert_eq!(file.repo, "ivar");
        assert_eq!(file.path, "src/main.rs");
        assert_eq!(file.content_hash, "hash_abc");

        // Re-upsert file updates existing row
        let file_id_2 = db
            .upsert_file("ivar", "src/main.rs", "hash_def", 2000, 4096)
            .expect("upsert updated file");
        assert_eq!(file_id, file_id_2);

        let file2 = db
            .get_file("ivar", "src/main.rs")
            .expect("get file")
            .expect("exists");
        assert_eq!(file2.content_hash, "hash_def");
        assert_eq!(file2.size_bytes, 4096);
    }

    #[test]
    fn test_foreign_key_cascades() {
        let db = GraphDb::open_in_memory().expect("open");
        db.insert_repo("ivar", "/path", "main", None).unwrap();
        let file_id = db
            .upsert_file("ivar", "src/lib.rs", "hash1", 1, 100)
            .unwrap();

        let sym = Symbol {
            id: None,
            file_id: Some(file_id),
            repo: "ivar".to_string(),
            name: "init".to_string(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("pub fn init()".to_string()),
            docstring: Some("Initializes the system.".to_string()),
            span: Span::new(1, 1, 10, 1),
            is_exported: true,
        };
        let sym_ids = db.insert_symbols(&[sym]).unwrap();
        let sym_id = sym_ids[0];

        let edge = Edge {
            id: None,
            repo: "ivar".to_string(),
            file_id: Some(file_id),
            from_symbol_id: Some(sym_id),
            to_symbol_id: None,
            to_name: Some("sub_init".to_string()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 5,
            col: 9,
            confidence: 1.0,
        };
        db.insert_edges(&[edge]).unwrap();

        let stats = db.stats().unwrap();
        assert_eq!(stats.file_count, 1);
        assert_eq!(stats.symbol_count, 1);
        assert_eq!(stats.edge_count, 1);

        // Deleting file should cascade delete symbol and edge
        db.delete_file("ivar", "src/lib.rs").unwrap();

        let stats_after = db.stats().unwrap();
        assert_eq!(stats_after.file_count, 0);
        assert_eq!(stats_after.symbol_count, 0);
        assert_eq!(stats_after.edge_count, 0);
    }

    #[test]
    fn test_delete_symbols_for_file() {
        let db = GraphDb::open_in_memory().expect("open");
        db.insert_repo("ivar", "/path", "main", None).unwrap();
        let file_id = db
            .upsert_file("ivar", "src/lib.rs", "hash1", 1, 100)
            .unwrap();

        let sym = Symbol {
            id: None,
            file_id: Some(file_id),
            repo: "ivar".to_string(),
            name: "test_fn".to_string(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: false,
        };
        let sym_ids = db.insert_symbols(&[sym]).unwrap();
        let sym_id = sym_ids[0];

        let edge = Edge {
            id: None,
            repo: "ivar".to_string(),
            file_id: Some(file_id),
            from_symbol_id: Some(sym_id),
            to_symbol_id: None,
            to_name: Some("callee".to_string()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 2,
            col: 5,
            confidence: 1.0,
        };
        db.insert_edges(&[edge]).unwrap();

        db.delete_symbols_for_file(file_id).unwrap();

        let stats = db.stats().unwrap();
        assert_eq!(stats.file_count, 1);
        assert_eq!(stats.symbol_count, 0);
        assert_eq!(stats.edge_count, 0);
    }

    #[test]
    fn test_fts5_triggers_and_search() {
        let db = GraphDb::open_in_memory().expect("open");
        db.insert_repo("ivar", "/path", "main", None).unwrap();
        let file_id = db
            .upsert_file("ivar", "src/service.rs", "hash_srv", 1, 500)
            .unwrap();

        let sym = Symbol {
            id: None,
            file_id: Some(file_id),
            repo: "ivar".to_string(),
            name: "compute_hash".to_string(),
            kind: SymbolKind::Fn,
            scope: Some("crate::service".to_string()),
            signature: Some("pub fn compute_hash(data: &[u8]) -> String".to_string()),
            docstring: Some("Computes SHA256 digest of input payload.".to_string()),
            span: Span::new(10, 1, 25, 1),
            is_exported: true,
        };
        db.insert_symbols(&[sym]).unwrap();

        // Search by symbol name
        let results = db.search_symbols_fts("compute_hash", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "compute_hash");
        assert_eq!(results[0].kind, SymbolKind::Fn);
        assert!(results[0].is_exported);

        // Search by docstring token
        let results_doc = db.search_symbols_fts("SHA256", 10).unwrap();
        assert_eq!(results_doc.len(), 1);
        assert_eq!(results_doc[0].name, "compute_hash");

        // Search by signature token
        let results_sig = db.search_symbols_fts("payload", 10).unwrap();
        assert_eq!(results_sig.len(), 1);

        // Delete symbol and check FTS index updated
        db.delete_symbols_for_file(file_id).unwrap();
        let results_after = db.search_symbols_fts("compute_hash", 10).unwrap();
        assert_eq!(results_after.len(), 0);
    }

    #[test]
    fn test_relink_dangling_edges() {
        let db = GraphDb::open_in_memory().expect("open");
        db.insert_repo("ivar", "/path", "main", None).unwrap();
        let file_caller = db
            .upsert_file("ivar", "src/caller.rs", "h1", 1, 100)
            .unwrap();
        let file_callee = db
            .upsert_file("ivar", "src/callee.rs", "h2", 1, 100)
            .unwrap();

        // Insert edge before callee symbol exists (dangling edge with to_symbol_id = NULL)
        let edge = Edge {
            id: None,
            repo: "ivar".to_string(),
            file_id: Some(file_caller),
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("target_fn".to_string()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 4,
            col: 10,
            confidence: 1.0,
        };
        db.insert_edges(&[edge]).unwrap();

        // Now insert target symbol
        let target_sym = Symbol {
            id: None,
            file_id: Some(file_callee),
            repo: "ivar".to_string(),
            name: "target_fn".to_string(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("pub fn target_fn()".to_string()),
            docstring: None,
            span: Span::new(1, 1, 10, 1),
            is_exported: true,
        };
        let sym_ids = db.insert_symbols(&[target_sym]).unwrap();
        let target_sym_id = sym_ids[0];

        let relinked = db.relink_dangling_edges("ivar").unwrap();
        assert_eq!(relinked, 1);

        // Verify edge now points to target_sym_id
        let to_sym_id: Option<i64> = db
            .conn()
            .query_row(
                "SELECT to_symbol_id FROM edges WHERE repo = 'ivar' AND to_name = 'target_fn'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(to_sym_id, Some(target_sym_id));
    }
}
