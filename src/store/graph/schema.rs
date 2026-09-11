//! SQLite schema, DDL migration definitions, and pragma configuration for the codebase graph.

use rusqlite::{Connection, params};

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
    is_exported INTEGER NOT NULL DEFAULT 0,
    name_words TEXT
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

-- Covered Indexes for <1ms Graph Traversal
CREATE INDEX IF NOT EXISTS idx_symbols_repo_name ON symbols(repo, name);
CREATE INDEX IF NOT EXISTS idx_symbols_file_id ON symbols(file_id);
CREATE INDEX IF NOT EXISTS idx_edges_from_symbol ON edges(from_symbol_id);
CREATE INDEX IF NOT EXISTS idx_edges_to_symbol ON edges(to_symbol_id);
CREATE INDEX IF NOT EXISTS idx_edges_to_name ON edges(to_name) WHERE to_symbol_id IS NULL;
CREATE INDEX IF NOT EXISTS idx_files_repo_hash ON files(repo, content_hash);
CREATE INDEX IF NOT EXISTS idx_files_repo ON files(repo);
CREATE INDEX IF NOT EXISTS idx_edges_file_id ON edges(file_id);
CREATE INDEX IF NOT EXISTS idx_edges_repo ON edges(repo);
CREATE INDEX IF NOT EXISTS idx_edges_provenance ON edges(provenance);
CREATE INDEX IF NOT EXISTS idx_symbols_name_repo ON symbols(name, repo);
CREATE INDEX IF NOT EXISTS idx_symbols_scope_repo ON symbols(scope, repo) WHERE scope IS NOT NULL;
"#;

/// Full-text index over symbols. The tokenizer keeps `enforceSession` as one
/// token, so `name_words` carries the identifier split into words.
const SYMBOLS_FTS: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
    name,
    name_words,
    scope,
    signature,
    docstring,
    content='symbols',
    content_rowid='id'
);

CREATE TRIGGER IF NOT EXISTS symbols_ai AFTER INSERT ON symbols BEGIN
    INSERT INTO symbols_fts(rowid, name, name_words, scope, signature, docstring)
    VALUES (new.id, new.name, new.name_words, new.scope, new.signature, new.docstring);
END;

CREATE TRIGGER IF NOT EXISTS symbols_ad AFTER DELETE ON symbols BEGIN
    INSERT INTO symbols_fts(symbols_fts, rowid, name, name_words, scope, signature, docstring)
    VALUES('delete', old.id, old.name, old.name_words, old.scope, old.signature, old.docstring);
END;

CREATE TRIGGER IF NOT EXISTS symbols_au AFTER UPDATE ON symbols BEGIN
    INSERT INTO symbols_fts(symbols_fts, rowid, name, name_words, scope, signature, docstring)
    VALUES('delete', old.id, old.name, old.name_words, old.scope, old.signature, old.docstring);
    INSERT INTO symbols_fts(rowid, name, name_words, scope, signature, docstring)
    VALUES (new.id, new.name, new.name_words, new.scope, new.signature, new.docstring);
END;
"#;

/// Databases below this `user_version` lack word search, and their rows predate
/// import references, JSX and HTTP client edges, so every file is re-extracted.
const SEARCH_SCHEMA_VERSION: i64 = 4;

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

    // Migration V2: Ensure complexity column exists on symbols table for existing DBs
    if !has_column(conn, "symbols", "complexity")? {
        conn.execute_batch("ALTER TABLE symbols ADD COLUMN complexity INTEGER;")?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_symbols_complexity ON symbols(complexity) WHERE complexity IS NOT NULL;",
    )?;

    apply_search_migration(conn)?;
    apply_layer_migration(conn)
}

fn apply_layer_migration(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS layers (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            feature TEXT NOT NULL,
            repo TEXT NOT NULL,
            worktree TEXT NOT NULL,
            base_commit TEXT NOT NULL,
            head_commit TEXT,
            fingerprint TEXT,
            indexed_at INTEGER NOT NULL,
            UNIQUE(feature, repo)
        );

        CREATE TABLE IF NOT EXISTS layer_tombstones (
            layer_id INTEGER NOT NULL REFERENCES layers(id) ON DELETE CASCADE,
            path TEXT NOT NULL,
            PRIMARY KEY(layer_id, path)
        );"
    )?;
    let version: i64 = conn.query_row("PRAGMA user_version;", [], |row| row.get(0))?;
    if version < SEARCH_SCHEMA_VERSION {
        conn.execute_batch(&format!("PRAGMA user_version = {SEARCH_SCHEMA_VERSION};"))?;
    }
    Ok(())
}

fn apply_search_migration(conn: &Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version;", [], |row| row.get(0))?;
    if version >= SEARCH_SCHEMA_VERSION {
        return Ok(());
    }

    let tx = conn.unchecked_transaction()?;
    // External-content FTS deletes must repeat the indexed values, so the words
    // are backfilled while no trigger and no index exist.
    tx.execute_batch(
        "DROP TRIGGER IF EXISTS symbols_ai;
         DROP TRIGGER IF EXISTS symbols_ad;
         DROP TRIGGER IF EXISTS symbols_au;
         DROP TABLE IF EXISTS symbols_fts;",
    )?;
    if !has_column(&tx, "symbols", "name_words")? {
        tx.execute_batch("ALTER TABLE symbols ADD COLUMN name_words TEXT;")?;
    }
    let names: Vec<(i64, String)> = tx
        .prepare("SELECT id, name FROM symbols")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    {
        let mut update = tx.prepare("UPDATE symbols SET name_words = ?1 WHERE id = ?2")?;
        for (id, name) in &names {
            update.execute(params![name_words(name), id])?;
        }
    }
    tx.execute_batch(SYMBOLS_FTS)?;
    tx.execute_batch(&format!(
        "INSERT INTO symbols_fts(symbols_fts) VALUES('rebuild');
         UPDATE files SET content_hash = '', mtime_ns = -1;
         UPDATE repos SET last_indexed_commit = NULL;
         PRAGMA user_version = {SEARCH_SCHEMA_VERSION};"
    ))?;
    tx.commit()
}

fn has_column(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table});"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Splits an identifier into lowercase words, so a search for "session" finds
/// `enforceSession`, `session_store` and `GET /sessions/:id`.
pub fn name_words(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for (index, &ch) in chars.iter().enumerate() {
        if !ch.is_alphanumeric() {
            words.extend((!current.is_empty()).then(|| std::mem::take(&mut current)));
            continue;
        }
        let prev = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(index + 1).copied();
        let starts_word = ch.is_uppercase()
            && prev.is_some_and(|p| {
                p.is_lowercase()
                    || p.is_ascii_digit()
                    || (p.is_uppercase() && next.is_some_and(char::is_lowercase))
            });
        if starts_word && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        current.extend(ch.to_lowercase());
    }
    words.extend((!current.is_empty()).then_some(current));
    words.join(" ")
}

#[cfg(test)]
#[path = "../../../tests/unit/store/graph/schema.rs"]
mod tests;
