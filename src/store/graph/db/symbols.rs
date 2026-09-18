//! Symbol table storage and full-text search operations.

use rusqlite::params;

use super::GraphDb;
use super::row;
use super::types::{Result, symbol_kind_to_str};
use crate::domain::graph::Symbol;
use crate::store::graph::schema::name_words;

impl GraphDb {
    /// Deletes all symbols belonging to a specific file ID (cascades to outbound edges).
    pub fn delete_symbols_for_file(&self, file_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM symbols WHERE file_id = ?1", params![file_id])?;
        Ok(())
    }

    /// Bulk inserts symbols in a single transaction and returns their generated IDs.
    pub fn insert_symbols(&self, symbols: &[Symbol]) -> Result<Vec<i64>> {
        if symbols.is_empty() {
            return Ok(Vec::new());
        }
        self.in_transaction(|| -> Result<Vec<i64>> {
            let mut stmt = self.conn.prepare_cached(
                "INSERT INTO symbols (file_id, repo, name, kind, scope, signature, docstring, start_line, start_col, end_line, end_col, is_exported, complexity, name_words)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 RETURNING id",
            )?;
            let mut ids = Vec::with_capacity(symbols.len());
            for sym in symbols {
                let kind_str = symbol_kind_to_str(&sym.kind);
                let is_exported = if sym.is_exported { 1 } else { 0 };
                let id: i64 = stmt.query_row(
                    params![
                        sym.file_id,
                        &sym.repo,
                        &sym.name,
                        kind_str.as_ref(),
                        &sym.scope,
                        &sym.signature,
                        &sym.docstring,
                        i64::try_from(sym.span.start_line).unwrap_or(i64::MAX),
                        i64::try_from(sym.span.start_col).unwrap_or(i64::MAX),
                        i64::try_from(sym.span.end_line).unwrap_or(i64::MAX),
                        i64::try_from(sym.span.end_col).unwrap_or(i64::MAX),
                        is_exported,
                        sym.complexity.map(i64::from),
                        name_words(&sym.name),
                    ],
                    |row| row.get(0),
                )?;
                ids.push(id);
            }
            Ok(ids)
        })
    }

    /// Full-text searches indexed symbols across all repositories using SQLite FTS5.
    pub fn search_symbols_fts(&self, query: &str, limit: usize) -> Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                    s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity
             FROM symbols_fts fts
             JOIN visible_symbols s ON fts.rowid = s.id
             WHERE symbols_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![query, i64::try_from(limit).unwrap_or(i64::MAX)],
            row::symbol_from_row,
        )?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Finds potentially unused/unreachable functions and methods with 0 callers.
    pub fn find_dead_code(
        &self,
        repo: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(Symbol, String)>> {
        let sql = match repo {
            Some(_) => {
                "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                        s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity,
                        f.path
                 FROM visible_symbols s
                 JOIN visible_files f ON s.file_id = f.id
                 WHERE s.repo = ?1
                   AND s.is_exported = 0
                   AND s.kind IN ('fn', 'method')
                   AND s.name NOT IN ('main', 'run', 'start', 'init', 'test', 'new')
                   AND s.name NOT LIKE 'test_%'
                   AND s.name NOT LIKE '%_test'
                   AND NOT EXISTS (
                       SELECT 1 FROM visible_edges e
                       WHERE e.to_symbol_id = s.id
                         AND e.kind IN ('CALLS', 'IMPLEMENTS', 'INHERITS')
                   )
                   AND NOT EXISTS (
                       SELECT 1 FROM visible_edges e
                       WHERE e.to_name = s.name
                         AND e.kind IN ('CALLS', 'IMPLEMENTS', 'INHERITS')
                   )
                 ORDER BY s.repo, f.path, s.start_line
                 LIMIT ?2"
            }
            None => {
                "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                        s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity,
                        f.path
                 FROM visible_symbols s
                 JOIN visible_files f ON s.file_id = f.id
                 WHERE s.is_exported = 0
                   AND s.kind IN ('fn', 'method')
                   AND s.name NOT IN ('main', 'run', 'start', 'init', 'test', 'new')
                   AND s.name NOT LIKE 'test_%'
                   AND s.name NOT LIKE '%_test'
                   AND NOT EXISTS (
                       SELECT 1 FROM visible_edges e
                       WHERE e.to_symbol_id = s.id
                         AND e.kind IN ('CALLS', 'IMPLEMENTS', 'INHERITS')
                   )
                   AND NOT EXISTS (
                       SELECT 1 FROM visible_edges e
                       WHERE e.to_name = s.name
                         AND e.kind IN ('CALLS', 'IMPLEMENTS', 'INHERITS')
                   )
                 ORDER BY s.repo, f.path, s.start_line
                 LIMIT ?1"
            }
        };

        let mut stmt = self.conn.prepare(sql)?;
        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<(Symbol, String)> {
            let symbol = row::symbol_from_row(row)?;
            let path: String = row.get(14)?;
            Ok((symbol, path))
        };

        let mut results = Vec::new();
        if let Some(r) = repo {
            let rows = stmt.query_map(params![r, i64::try_from(limit).unwrap_or(i64::MAX)], map_row)?;
            for row in rows {
                results.push(row?);
            }
        } else {
            let rows = stmt.query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], map_row)?;
            for row in rows {
                results.push(row?);
            }
        }
        Ok(results)
    }

    /// Queries symbols sorted by cyclomatic complexity descending.
    pub fn find_complex_symbols(
        &self,
        repo: Option<&str>,
        threshold: u32,
        limit: usize,
    ) -> Result<Vec<(Symbol, String)>> {
        let sql = match repo {
            Some(_) => {
                "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                        s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity,
                        f.path
                 FROM visible_symbols s
                 JOIN visible_files f ON s.file_id = f.id
                 WHERE s.complexity >= ?1 AND s.repo = ?2
                 ORDER BY s.complexity DESC, s.name ASC
                 LIMIT ?3"
            }
            None => {
                "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                        s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity,
                        f.path
                 FROM visible_symbols s
                 JOIN visible_files f ON s.file_id = f.id
                 WHERE s.complexity >= ?1
                 ORDER BY s.complexity DESC, s.name ASC
                 LIMIT ?2"
            }
        };

        let mut stmt = self.conn.prepare(sql)?;
        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<(Symbol, String)> {
            let symbol = row::symbol_from_row(row)?;
            let path: String = row.get(14)?;
            Ok((symbol, path))
        };

        let mut results = Vec::new();
        if let Some(r) = repo {
            let rows = stmt.query_map(params![i64::from(threshold), r, i64::try_from(limit).unwrap_or(i64::MAX)], map_row)?;
            for row in rows {
                results.push(row?);
            }
        } else {
            let rows = stmt.query_map(params![i64::from(threshold), i64::try_from(limit).unwrap_or(i64::MAX)], map_row)?;
            for row in rows {
                results.push(row?);
            }
        }
        Ok(results)
    }
}
