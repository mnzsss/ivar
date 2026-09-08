//! SQLite FTS5 derived index for shared memory topic documents.
//!
//! Provides transactional hash-based reconciliation, BM25-ranked full-text search,
//! snippet generation, and transparent recovery from database corruption.

use std::collections::HashMap;

use camino::{Utf8Path, Utf8PathBuf};
use rusqlite::{Connection, OpenFlags, params};

use crate::domain::memory::config::ScopeName;
use crate::domain::memory::query::{QueryFilter, QueryMatch, ReconcileSummary};
use crate::error::{Failure, FixAction};
use crate::infra::fs;
use crate::infra::hash;
use crate::store::layout::Layout;
use crate::store::memory::document::read_topic;

/// Disposable SQLite FTS5 derived index for memory topics.
#[derive(Debug, Clone)]
pub struct MemoryIndex {
    db_path: Utf8PathBuf,
}

/// Top-level helper to reconcile or rebuild the SQLite FTS index.
pub fn reconcile_fts_index(layout: &Layout, rebuild: bool) -> Result<ReconcileSummary, Failure> {
    let index = MemoryIndex::open(layout)?;
    if rebuild {
        index.rebuild(layout)
    } else {
        index.reconcile(layout)
    }
}

impl MemoryIndex {
    /// Open or create the memory index at `layout.memory_index_db()`.
    pub fn open(layout: &Layout) -> Result<Self, Failure> {
        let db_path = layout.memory_index_db();
        Self::init_db(&db_path)?;
        Ok(Self { db_path })
    }

    /// Reconcile the SQLite index against canonical markdown files on disk.
    pub fn reconcile(&self, layout: &Layout) -> Result<ReconcileSummary, Failure> {
        match self.reconcile_inner(layout) {
            Ok(summary) => Ok(summary),
            Err(err) if Self::is_corruption_error(&err) => self.rebuild(layout),
            Err(err) => Err(err),
        }
    }

    /// Execute a full-text search query across indexed documents.
    pub fn query(&self, query: &str, filter: &QueryFilter) -> Result<Vec<QueryMatch>, Failure> {
        let formatted_query = format_fts5_query(query);
        if formatted_query.is_empty() {
            return Ok(Vec::new());
        }

        let conn = Self::connect(&self.db_path)?;

        let mut results = Vec::new();
        if let Some(scope) = &filter.scope {
            let mut stmt = conn
                .prepare(
                    "SELECT scope, slug, path, title, snippet(documents_fts, -1, '', '', '...', 20), rank
                     FROM documents_fts
                     WHERE documents_fts MATCH ?1 AND scope = ?2
                     ORDER BY rank
                     LIMIT ?3",
                )
                .map_err(|err| sqlite_failure("query_prepare", err))?;

            let rows = stmt
                .query_map(
                    params![&formatted_query, scope.as_str(), filter.limit as i64],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, f64>(5)?,
                        ))
                    },
                )
                .map_err(|err| sqlite_failure("query_execute", err))?;

            for row in rows {
                let (scope_str, slug, path, title, snippet, rank) =
                    row.map_err(|err| sqlite_failure("query_row", err))?;
                let match_scope = ScopeName::new(scope_str)?;
                results.push(QueryMatch {
                    scope: match_scope,
                    slug,
                    path,
                    title,
                    snippet,
                    rank,
                });
            }
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT scope, slug, path, title, snippet(documents_fts, -1, '', '', '...', 20), rank
                     FROM documents_fts
                     WHERE documents_fts MATCH ?1
                     ORDER BY rank
                     LIMIT ?2",
                )
                .map_err(|err| sqlite_failure("query_prepare", err))?;

            let rows = stmt
                .query_map(params![&formatted_query, filter.limit as i64], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, f64>(5)?,
                    ))
                })
                .map_err(|err| sqlite_failure("query_execute", err))?;

            for row in rows {
                let (scope_str, slug, path, title, snippet, rank) =
                    row.map_err(|err| sqlite_failure("query_row", err))?;
                let match_scope = ScopeName::new(scope_str)?;
                results.push(QueryMatch {
                    scope: match_scope,
                    slug,
                    path,
                    title,
                    snippet,
                    rank,
                });
            }
        }

        Ok(results)
    }

    /// Rebuild the index from scratch by removing the existing database and re-indexing.
    pub fn rebuild(&self, layout: &Layout) -> Result<ReconcileSummary, Failure> {
        Self::remove_db_files(&self.db_path)?;
        Self::init_db(&self.db_path)?;
        self.reconcile_inner(layout)
    }

    fn reconcile_inner(&self, layout: &Layout) -> Result<ReconcileSummary, Failure> {
        let mut conn = Self::connect(&self.db_path)?;
        let tx = conn
            .transaction()
            .map_err(|err| sqlite_failure("transaction_begin", err))?;

        // Ensure tables exist in transaction
        init_schema(&tx)?;

        let mut existing = HashMap::new();
        {
            let mut stmt = tx
                .prepare("SELECT scope, slug, content_hash FROM documents")
                .map_err(|err| sqlite_failure("select_documents", err))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|err| sqlite_failure("query_documents", err))?;

            for row in rows {
                let (scope, slug, hash) =
                    row.map_err(|err| sqlite_failure("read_document_row", err))?;
                existing.insert((scope, slug), hash);
            }
        }

        let mut indexed = 0;
        let mut updated = 0;
        let mut removed = 0;

        let memory_root = layout.memory_root();
        if fs::exists(&memory_root).map_err(Into::<Failure>::into)? {
            let entries = fs::read_dir(&memory_root).map_err(Into::<Failure>::into)?;
            for scope_path in entries {
                let scope_name_str = match scope_path.file_name() {
                    Some(name) => name,
                    None => continue,
                };
                if scope_name_str.starts_with('.') {
                    continue;
                }

                let scope = match ScopeName::new(scope_name_str) {
                    Ok(scope) => scope,
                    Err(_) => continue,
                };

                let file_entries = match fs::read_dir(&scope_path) {
                    Ok(entries) => entries,
                    Err(_) => continue,
                };

                for file_path in file_entries {
                    let file_name = match file_path.file_name() {
                        Some(name) => name,
                        None => continue,
                    };
                    if file_name.starts_with('.') || !file_name.ends_with(".md") {
                        continue;
                    }
                    let slug = match file_name.strip_suffix(".md") {
                        Some(slug) if !slug.is_empty() => slug,
                        _ => continue,
                    };

                    let topic = read_topic(layout, &scope, slug)?;
                    let content_hash = hash::file(&file_path).map_err(Into::<Failure>::into)?;
                    let rel_path = format!("memory/{}/{}.md", scope.as_str(), slug);
                    let key = (scope.as_str().to_owned(), slug.to_owned());

                    if let Some(old_hash) = existing.remove(&key) {
                        if old_hash != content_hash {
                            tx.execute(
                                "UPDATE documents SET path = ?1, title = ?2, content_hash = ?3, updated_at = ?4 WHERE scope = ?5 AND slug = ?6",
                                params![
                                    &rel_path,
                                    &topic.metadata.title,
                                    &content_hash,
                                    &topic.metadata.updated,
                                    scope.as_str(),
                                    slug,
                                ],
                            )
                            .map_err(|err| sqlite_failure("update_document", err))?;

                            tx.execute(
                                "DELETE FROM documents_fts WHERE scope = ?1 AND slug = ?2",
                                params![scope.as_str(), slug],
                            )
                            .map_err(|err| sqlite_failure("delete_fts_document", err))?;

                            tx.execute(
                                "INSERT INTO documents_fts (title, content, scope, slug, path) VALUES (?1, ?2, ?3, ?4, ?5)",
                                params![
                                    &topic.metadata.title,
                                    &topic.content,
                                    scope.as_str(),
                                    slug,
                                    &rel_path,
                                ],
                            )
                            .map_err(|err| sqlite_failure("insert_fts_document", err))?;

                            updated += 1;
                        }
                    } else {
                        tx.execute(
                            "INSERT INTO documents (scope, slug, path, title, content_hash, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                            params![
                                scope.as_str(),
                                slug,
                                &rel_path,
                                &topic.metadata.title,
                                &content_hash,
                                &topic.metadata.updated,
                            ],
                        )
                        .map_err(|err| sqlite_failure("insert_document", err))?;

                        tx.execute(
                            "INSERT INTO documents_fts (title, content, scope, slug, path) VALUES (?1, ?2, ?3, ?4, ?5)",
                            params![
                                &topic.metadata.title,
                                &topic.content,
                                scope.as_str(),
                                slug,
                                &rel_path,
                            ],
                        )
                        .map_err(|err| sqlite_failure("insert_fts_document", err))?;

                        indexed += 1;
                    }
                }
            }
        }

        for ((scope_str, slug_str), _) in existing {
            tx.execute(
                "DELETE FROM documents WHERE scope = ?1 AND slug = ?2",
                params![&scope_str, &slug_str],
            )
            .map_err(|err| sqlite_failure("delete_missing_document", err))?;

            tx.execute(
                "DELETE FROM documents_fts WHERE scope = ?1 AND slug = ?2",
                params![&scope_str, &slug_str],
            )
            .map_err(|err| sqlite_failure("delete_missing_fts_document", err))?;

            removed += 1;
        }

        tx.commit()
            .map_err(|err| sqlite_failure("transaction_commit", err))?;

        Ok(ReconcileSummary {
            indexed,
            updated,
            removed,
        })
    }

    fn init_db(db_path: &Utf8Path) -> Result<(), Failure> {
        if let Some(parent) = db_path.parent() {
            fs::ensure_dir(parent).map_err(Into::<Failure>::into)?;
        }

        let conn = match Self::connect(db_path) {
            Ok(conn) => conn,
            Err(err) => {
                if Self::is_corruption_error(&err) {
                    Self::remove_db_files(db_path)?;
                    Self::connect(db_path)?
                } else {
                    return Err(err);
                }
            }
        };

        if let Err(err) = init_schema(&conn) {
            if Self::is_corruption_error(&err) {
                drop(conn);
                Self::remove_db_files(db_path)?;
                let fresh_conn = Self::connect(db_path)?;
                init_schema(&fresh_conn)?;
            } else {
                return Err(err);
            }
        }

        Ok(())
    }

    fn connect(db_path: &Utf8Path) -> Result<Connection, Failure> {
        let conn = Connection::open_with_flags(
            db_path.as_std_path(),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|err| sqlite_failure("db_open", err))?;

        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))
            .map_err(|err| sqlite_failure("pragma_journal_mode", err))?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|err| sqlite_failure("pragma_synchronous", err))?;

        Ok(conn)
    }

    fn remove_db_files(db_path: &Utf8Path) -> Result<(), Failure> {
        let _ = fs::remove_file(db_path);
        let wal_path = Utf8PathBuf::from(format!("{db_path}-wal"));
        let _ = fs::remove_file(&wal_path);
        let shm_path = Utf8PathBuf::from(format!("{db_path}-shm"));
        let _ = fs::remove_file(&shm_path);
        Ok(())
    }

    fn is_corruption_error(err: &Failure) -> bool {
        let msg = err.to_string().to_lowercase();
        msg.contains("corrupt")
            || msg.contains("malformed")
            || msg.contains("not a database")
            || msg.contains("disk i/o error")
    }
}

fn init_schema(conn: &Connection) -> Result<(), Failure> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS documents (
            scope TEXT NOT NULL,
            slug TEXT NOT NULL,
            path TEXT NOT NULL,
            title TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (scope, slug)
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(
            title,
            content,
            scope UNINDEXED,
            slug UNINDEXED,
            path UNINDEXED,
            tokenize = 'porter unicode61'
        );",
    )
    .map_err(|err| sqlite_failure("init_schema", err))?;

    Ok(())
}

fn format_fts5_query(raw: &str) -> String {
    let mut tokens = Vec::new();
    for part in raw.split_whitespace() {
        let trimmed = part.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != '"' && c != '*' && c != '-' && c != '_'
        });
        if trimmed.is_empty() {
            continue;
        }
        let escaped = trimmed.replace('"', "\"\"");
        tokens.push(format!("\"{escaped}\""));
    }
    tokens.join(" ")
}
fn sqlite_failure(op: &str, err: rusqlite::Error) -> Failure {
    Failure::failed(
        "memory_index.sqlite_error",
        format!("sqlite error during {op}: {err}"),
    )
    .fix(FixAction::safe(
        "rebuild_memory_index",
        "Rebuild the derived memory index from canonical markdown files.",
    ))
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/index.rs"]
mod tests;
