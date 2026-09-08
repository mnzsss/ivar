//! SQLite schema, DDL migration definitions, and pragma configuration for the codebase graph.

use rusqlite::Connection;

/// Initial schema migration for the codebase graph (v1).
pub const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS repos (
    id TEXT PRIMARY KEY,
    root_path TEXT NOT NULL,
    default_branch TEXT NOT NULL,
    last_indexed_commit TEXT,
    indexed_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo TEXT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    mtime_ns INTEGER NOT NULL,
    size_bytes INTEGER NOT NULL,
    indexed_at INTEGER NOT NULL,
    UNIQUE(repo, path)
);

CREATE TABLE IF NOT EXISTS symbols (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    repo TEXT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    scope TEXT,
    signature TEXT,
    docstring TEXT,
    start_line INTEGER NOT NULL,
    start_col INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    end_col INTEGER NOT NULL,
    is_exported INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo TEXT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    from_symbol_id INTEGER REFERENCES symbols(id) ON DELETE CASCADE,
    to_symbol_id INTEGER REFERENCES symbols(id) ON DELETE SET NULL,
    to_name TEXT,
    kind TEXT NOT NULL,
    provenance TEXT NOT NULL DEFAULT 'EXTRACTED',
    line INTEGER NOT NULL,
    col INTEGER NOT NULL,
    confidence REAL NOT NULL DEFAULT 1.0
);

-- Full Text Search Virtual Table (FTS5 external content table)
CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
    name,
    scope,
    signature,
    docstring,
    content='symbols',
    content_rowid='id'
);

-- Triggers to maintain FTS index consistency
CREATE TRIGGER IF NOT EXISTS symbols_ai AFTER INSERT ON symbols BEGIN
    INSERT INTO symbols_fts(rowid, name, scope, signature, docstring)
    VALUES (new.id, new.name, new.scope, new.signature, new.docstring);
END;

CREATE TRIGGER IF NOT EXISTS symbols_ad AFTER DELETE ON symbols BEGIN
    INSERT INTO symbols_fts(symbols_fts, rowid, name, scope, signature, docstring)
    VALUES('delete', old.id, old.name, old.scope, old.signature, old.docstring);
END;

CREATE TRIGGER IF NOT EXISTS symbols_au AFTER UPDATE ON symbols BEGIN
    INSERT INTO symbols_fts(symbols_fts, rowid, name, scope, signature, docstring)
    VALUES('delete', old.id, old.name, old.scope, old.signature, old.docstring);
    INSERT INTO symbols_fts(rowid, name, scope, signature, docstring)
    VALUES (new.id, new.name, new.scope, new.signature, new.docstring);
END;

-- Covered Indexes for <1ms Graph Traversal
CREATE INDEX IF NOT EXISTS idx_symbols_repo_name ON symbols(repo, name);
CREATE INDEX IF NOT EXISTS idx_symbols_file_id ON symbols(file_id);
CREATE INDEX IF NOT EXISTS idx_edges_from_symbol ON edges(from_symbol_id);
CREATE INDEX IF NOT EXISTS idx_edges_to_symbol ON edges(to_symbol_id);
CREATE INDEX IF NOT EXISTS idx_edges_to_name ON edges(to_name) WHERE to_symbol_id IS NULL;
CREATE INDEX IF NOT EXISTS idx_files_repo_hash ON files(repo, content_hash);
CREATE INDEX IF NOT EXISTS idx_edges_provenance ON edges(provenance);
"#;

/// Configures SQLite pragmas for performance and data integrity.
pub fn apply_pragmas(conn: &Connection, is_disk: bool) -> rusqlite::Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    if is_disk {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -64000;",
        )?;
    }
    Ok(())
}

/// Applies initial database migrations and index creation.
pub fn apply_migrations(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(MIGRATION_V1)?;
    Ok(())
}
