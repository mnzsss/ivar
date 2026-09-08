//! Symbol table storage and full-text search operations.

use rusqlite::params;

use super::types::{Result, parse_symbol_kind, symbol_kind_to_str};
use super::GraphDb;
use crate::domain::graph::{Span, Symbol};

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
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let res = (|| -> Result<Vec<i64>> {
            let mut stmt = self.conn.prepare_cached(
                "INSERT INTO symbols (file_id, repo, name, kind, scope, signature, docstring, start_line, start_col, end_line, end_col, is_exported)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
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

    /// Full-text searches indexed symbols across all repositories using SQLite FTS5.
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
}
