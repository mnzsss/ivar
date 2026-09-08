//! Domain graph structures and graph traversal algorithms.
//!
//! Provides <1ms symbol discovery, callers/callees lookup, file outline,
//! graph stats, and transitive blast-radius impact analysis.

pub mod find;
pub mod impact;
pub mod types;

use rusqlite::{OptionalExtension, params};

pub use find::find_symbols;
pub use impact::get_impact;
pub use types::*;

use crate::domain::graph::{Edge, GraphStats, Span, Symbol};
use crate::store::graph::db::{GraphDb, parse_edge_kind, parse_provenance, parse_symbol_kind};

/// Retrieves all callers referencing the given symbol name.
pub fn get_callers(
    db: &GraphDb,
    symbol_name: &str,
    repo: Option<&str>,
    cross_repo: bool,
    min_confidence: f64,
) -> Result<Vec<CallerInfo>, QueryError> {
    let conn = db.conn();
    let mut stmt = conn.prepare_cached(
        "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, f.path,
                e.kind, e.provenance, e.confidence, e.line, e.col
         FROM edges e
         JOIN symbols s ON e.from_symbol_id = s.id
         JOIN files f ON s.file_id = f.id
         WHERE (
             e.to_symbol_id IN (
                 SELECT id FROM symbols WHERE name = ?1 AND (?2 IS NULL OR repo = ?2)
             )
             OR (
                 e.to_symbol_id IS NULL AND e.to_name = ?1 AND (?2 IS NULL OR e.repo = ?2)
             )
         )
         AND e.confidence >= ?3
         AND (?4 = 1 OR ?2 IS NULL OR s.repo = ?2)
         ORDER BY e.confidence DESC, s.name ASC",
    )?;

    let cross_repo_int = if cross_repo { 1 } else { 0 };
    let rows = stmt.query_map(
        params![symbol_name, repo, min_confidence, cross_repo_int],
        |row| {
            let (caller, caller_file_path) = map_symbol_and_path_row(row)?;
            let kind_raw: String = row.get(14)?;
            let provenance_raw: String = row.get(15)?;
            let confidence: f64 = row.get(16)?;
            let line: i64 = row.get(17)?;
            let col: i64 = row.get(18)?;

            Ok(CallerInfo {
                caller,
                caller_file_path,
                edge_kind: parse_edge_kind(&kind_raw),
                provenance: parse_provenance(&provenance_raw),
                confidence,
                line: line as usize,
                col: col as usize,
            })
        },
    )?;

    let mut callers = Vec::new();
    for r in rows {
        callers.push(r?);
    }
    Ok(callers)
}

/// Retrieves all outgoing calls/callees from a specific symbol ID.
pub fn get_callees(db: &GraphDb, symbol_id: i64) -> Result<Vec<CalleeInfo>, QueryError> {
    let conn = db.conn();
    let mut stmt = conn.prepare_cached(
        "SELECT
            e.to_name,
            s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
            s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported,
            f.path,
            e.kind, e.provenance, e.confidence, e.line, e.col
         FROM edges e
         LEFT JOIN symbols s ON e.to_symbol_id = s.id
         LEFT JOIN files f ON s.file_id = f.id
         WHERE e.from_symbol_id = ?1
         ORDER BY e.line ASC, e.col ASC",
    )?;

    let rows = stmt.query_map(params![symbol_id], |row| {
        let to_name: Option<String> = row.get(0)?;
        let sym_id: Option<i64> = row.get(1)?;
        let (callee_symbol, callee_file_path, callee_name) = if let Some(id) = sym_id {
            let file_id: i64 = row.get(2)?;
            let repo: String = row.get(3)?;
            let name: String = row.get(4)?;
            let kind_raw: String = row.get(5)?;
            let scope: Option<String> = row.get(6)?;
            let signature: Option<String> = row.get(7)?;
            let docstring: Option<String> = row.get(8)?;
            let start_line: i64 = row.get(9)?;
            let start_col: i64 = row.get(10)?;
            let end_line: i64 = row.get(11)?;
            let end_col: i64 = row.get(12)?;
            let is_exported: i64 = row.get(13)?;
            let path: String = row.get(14)?;

            let sym = Symbol {
                id: Some(id),
                file_id: Some(file_id),
                repo,
                name: name.clone(),
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
                complexity: None,
            };
            (Some(sym), Some(path), name)
        } else {
            let name = to_name.unwrap_or_default();
            (None, None, name)
        };

        let kind_raw: String = row.get(15)?;
        let provenance_raw: String = row.get(16)?;
        let confidence: f64 = row.get(17)?;
        let line: i64 = row.get(18)?;
        let col: i64 = row.get(19)?;

        Ok(CalleeInfo {
            callee_name,
            callee_symbol,
            callee_file_path,
            edge_kind: parse_edge_kind(&kind_raw),
            provenance: parse_provenance(&provenance_raw),
            confidence,
            line: line as usize,
            col: col as usize,
        })
    })?;

    let mut callees = Vec::new();
    for r in rows {
        callees.push(r?);
    }
    Ok(callees)
}

/// Retrieves symbols and imports for a single file outline.
pub fn get_file_outline(db: &GraphDb, repo: &str, path: &str) -> Result<FileOutline, QueryError> {
    let conn = db.conn();

    // 1. Fetch file ID
    let mut file_stmt =
        conn.prepare_cached("SELECT id FROM files WHERE repo = ?1 AND path = ?2")?;
    let file_id: i64 = file_stmt
        .query_row(params![repo, path], |row| row.get(0))
        .optional()?
        .ok_or_else(|| QueryError::FileNotFound {
            repo: repo.to_owned(),
            path: path.to_owned(),
        })?;

    // 2. Fetch all symbols in this file
    let mut sym_stmt = conn.prepare_cached(
        "SELECT id, file_id, repo, name, kind, scope, signature, docstring,
                start_line, start_col, end_line, end_col, is_exported
         FROM symbols
         WHERE file_id = ?1
         ORDER BY start_line ASC, start_col ASC",
    )?;
    let symbols = sym_stmt
        .query_map(params![file_id], map_symbol_row)?
        .collect::<Result<Vec<_>, _>>()?;

    // 3. Fetch import edges
    let mut edge_stmt = conn.prepare_cached(
        "SELECT id, repo, file_id, from_symbol_id, to_symbol_id, to_name,
                kind, provenance, line, col, confidence
         FROM edges
         WHERE file_id = ?1
           AND kind IN ('IMPORTS', 'CROSS_IMPORTS', 'imports', 'cross_imports')
         ORDER BY line ASC, col ASC",
    )?;
    let imports = edge_stmt
        .query_map(params![file_id], |row| {
            let id: i64 = row.get(0)?;
            let repo: String = row.get(1)?;
            let file_id: i64 = row.get(2)?;
            let from_symbol_id: Option<i64> = row.get(3)?;
            let to_symbol_id: Option<i64> = row.get(4)?;
            let to_name: Option<String> = row.get(5)?;
            let kind_raw: String = row.get(6)?;
            let provenance_raw: String = row.get(7)?;
            let line: i64 = row.get(8)?;
            let col: i64 = row.get(9)?;
            let confidence: f64 = row.get(10)?;

            Ok(Edge {
                id: Some(id),
                repo,
                file_id: Some(file_id),
                from_symbol_id,
                to_symbol_id,
                to_name,
                kind: parse_edge_kind(&kind_raw),
                provenance: parse_provenance(&provenance_raw),
                line: line as usize,
                col: col as usize,
                confidence,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(FileOutline {
        file_path: path.to_owned(),
        repo: repo.to_owned(),
        symbols,
        imports,
    })
}

/// Returns overall graph statistics.
pub fn get_graph_stats(db: &GraphDb) -> Result<GraphStats, QueryError> {
    db.stats().map_err(QueryError::from)
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/query.rs"]
mod tests;
