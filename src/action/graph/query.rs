//! Domain query structures and graph traversal algorithms.
//!
//! Provides <1ms symbol discovery, callers/callees lookup, file outline,
//! graph stats, and transitive blast-radius impact analysis.

use rusqlite::{params, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::graph::{
    Edge, EdgeKind, GraphStats, Provenance, Span, Symbol,
};
use crate::infra::graph::db::{
    parse_edge_kind, parse_provenance, parse_symbol_kind, GraphDb, GraphDbError,
};

/// Error returned during graph query execution.
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("symbol not found: {0}")]
    SymbolNotFound(String),
    #[error("file not found: repo '{repo}', path '{path}'")]
    FileNotFound { repo: String, path: String },
    #[error("database error: {0}")]
    Db(#[from] GraphDbError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Symbol with associated file path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SymbolLocation {
    pub symbol: Symbol,
    pub file_path: String,
}

/// Information about a caller referencing a symbol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CallerInfo {
    pub caller: Symbol,
    pub caller_file_path: String,
    pub edge_kind: EdgeKind,
    pub provenance: Provenance,
    pub confidence: f64,
    pub line: usize,
    pub col: usize,
}

/// Information about an outgoing call/target from a symbol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CalleeInfo {
    pub callee_name: String,
    pub callee_symbol: Option<Symbol>,
    pub callee_file_path: Option<String>,
    pub edge_kind: EdgeKind,
    pub provenance: Provenance,
    pub confidence: f64,
    pub line: usize,
    pub col: usize,
}

/// Structural outline of symbols and import dependencies in a single file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FileOutline {
    pub file_path: String,
    pub repo: String,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Edge>,
}

/// An affected symbol reached during blast-radius impact traversal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImpactItem {
    pub symbol: Symbol,
    pub file_path: String,
    pub depth: usize,
    pub path_via: Vec<String>,
}

/// Aggregate blast-radius impact result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImpactResult {
    pub root_symbol: Symbol,
    pub affected_symbols: Vec<ImpactItem>,
    pub affected_files: Vec<String>,
    pub total_affected: usize,
}

fn map_symbol_and_path_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(Symbol, String)> {
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
    let file_path: String = row.get(13)?;

    Ok((
        Symbol {
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
        },
        file_path,
    ))
}

fn map_symbol_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Symbol> {
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
}

/// Finds symbols by exact name, prefix, and full-text search.
pub fn find_symbols(
    db: &GraphDb,
    query: &str,
    repo: Option<&str>,
    limit: usize,
) -> Result<Vec<SymbolLocation>, QueryError> {
    if limit == 0 || query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let conn = db.conn();
    let mut results = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    // 1. Exact name matches
    {
        let mut stmt = conn.prepare_cached(
            "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                    s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, f.path
             FROM symbols s
             JOIN files f ON s.file_id = f.id
             WHERE s.name = ?1 AND (?2 IS NULL OR s.repo = ?2)
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![query, repo, limit as i64], map_symbol_and_path_row)?;
        for row in rows {
            let (sym, path) = row?;
            if let Some(id) = sym.id {
                if seen_ids.insert(id) {
                    results.push(SymbolLocation {
                        symbol: sym,
                        file_path: path,
                    });
                    if results.len() >= limit {
                        return Ok(results);
                    }
                }
            }
        }
    }

    // 2. Prefix matches
    if results.len() < limit {
        let prefix_query = format!("{query}%");
        let mut stmt = conn.prepare_cached(
            "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                    s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, f.path
             FROM symbols s
             JOIN files f ON s.file_id = f.id
             WHERE s.name LIKE ?1 AND (?2 IS NULL OR s.repo = ?2)
             ORDER BY length(s.name) ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![prefix_query, repo, limit as i64], map_symbol_and_path_row)?;
        for row in rows {
            let (sym, path) = row?;
            if let Some(id) = sym.id {
                if seen_ids.insert(id) {
                    results.push(SymbolLocation {
                        symbol: sym,
                        file_path: path,
                    });
                    if results.len() >= limit {
                        return Ok(results);
                    }
                }
            }
        }
    }

    // 3. FTS5 search
    if results.len() < limit {
        let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
        if let Ok(mut stmt) = conn.prepare_cached(
            "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                    s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, f.path
             FROM symbols_fts fts
             JOIN symbols s ON fts.rowid = s.id
             JOIN files f ON s.file_id = f.id
             WHERE symbols_fts MATCH ?1 AND (?2 IS NULL OR s.repo = ?2)
             ORDER BY rank
             LIMIT ?3",
        ) {
            if let Ok(rows) = stmt.query_map(params![fts_query, repo, limit as i64], map_symbol_and_path_row) {
                for row in rows.flatten() {
                    let (sym, path) = row;
                    if let Some(id) = sym.id {
                        if seen_ids.insert(id) {
                            results.push(SymbolLocation {
                                symbol: sym,
                                file_path: path,
                            });
                            if results.len() >= limit {
                                return Ok(results);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(results)
}

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
        "SELECT
            s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
            s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported,
            f.path,
            e.kind, e.provenance, e.confidence, e.line, e.col
         FROM edges e
         JOIN symbols s ON e.from_symbol_id = s.id
         JOIN files f ON s.file_id = f.id
         WHERE (
             e.to_symbol_id IN (
                 SELECT id FROM symbols WHERE name = ?1 AND (?2 IS NULL OR repo = ?2)
             )
             OR (
                 e.to_name = ?1 AND (?2 IS NULL OR e.repo = ?2)
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

/// Retrieves symbols and import edges for a given file.
pub fn get_file_outline(db: &GraphDb, repo: &str, path: &str) -> Result<FileOutline, QueryError> {
    let conn = db.conn();
    let file_id: i64 = conn
        .query_row(
            "SELECT id FROM files WHERE repo = ?1 AND path = ?2",
            params![repo, path],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| QueryError::FileNotFound {
            repo: repo.to_string(),
            path: path.to_string(),
        })?;

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
        file_path: path.to_string(),
        repo: repo.to_string(),
        symbols,
        imports,
    })
}

/// Returns overall graph statistics.
pub fn get_graph_stats(db: &GraphDb) -> Result<GraphStats, QueryError> {
    db.stats().map_err(QueryError::from)
}

/// Analyzes blast-radius impact of changing a symbol by recursively traversing callers.
pub fn get_impact(
    db: &GraphDb,
    symbol_id: i64,
    max_depth: usize,
) -> Result<ImpactResult, QueryError> {
    let conn = db.conn();
    let mut root_stmt = conn.prepare_cached(
        "SELECT id, file_id, repo, name, kind, scope, signature, docstring,
                start_line, start_col, end_line, end_col, is_exported
         FROM symbols
         WHERE id = ?1",
    )?;
    let root_symbol = root_stmt
        .query_row(params![symbol_id], map_symbol_row)
        .optional()?
        .ok_or_else(|| QueryError::SymbolNotFound(symbol_id.to_string()))?;

    if max_depth == 0 {
        return Ok(ImpactResult {
            root_symbol,
            affected_symbols: Vec::new(),
            affected_files: Vec::new(),
            total_affected: 0,
        });
    }

    let mut stmt = conn.prepare_cached(
        "WITH RECURSIVE caller_graph(symbol_id, symbol_name, depth, visited_ids, path_names) AS (
            SELECT s.id, s.name, 0, ',' || CAST(s.id AS TEXT) || ',', CAST(s.name AS TEXT)
            FROM symbols s WHERE s.id = ?1
            UNION
            SELECT
                e.from_symbol_id,
                s.name,
                cg.depth + 1,
                cg.visited_ids || CAST(s.id AS TEXT) || ',',
                cg.path_names || ' -> ' || s.name
            FROM edges e
            JOIN caller_graph cg ON (e.to_symbol_id = cg.symbol_id OR e.to_name = cg.symbol_name)
            JOIN symbols s ON e.from_symbol_id = s.id
            WHERE cg.depth < ?2
              AND e.from_symbol_id IS NOT NULL
              AND instr(cg.visited_ids, ',' || CAST(s.id AS TEXT) || ',') = 0
        )
        SELECT cg.symbol_id, cg.depth, cg.path_names,
               s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
               s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported,
               f.path
        FROM caller_graph cg
        JOIN symbols s ON cg.symbol_id = s.id
        JOIN files f ON s.file_id = f.id
        WHERE cg.symbol_id != ?1
        ORDER BY cg.depth ASC, s.name ASC",
    )?;

    let rows = stmt.query_map(params![symbol_id, max_depth as i64], |row| {
        let depth: i64 = row.get(1)?;
        let path_names: String = row.get(2)?;
        let id: i64 = row.get(3)?;
        let file_id: i64 = row.get(4)?;
        let repo: String = row.get(5)?;
        let name: String = row.get(6)?;
        let kind_raw: String = row.get(7)?;
        let scope: Option<String> = row.get(8)?;
        let signature: Option<String> = row.get(9)?;
        let docstring: Option<String> = row.get(10)?;
        let start_line: i64 = row.get(11)?;
        let start_col: i64 = row.get(12)?;
        let end_line: i64 = row.get(13)?;
        let end_col: i64 = row.get(14)?;
        let is_exported: i64 = row.get(15)?;
        let file_path: String = row.get(16)?;

        let sym = Symbol {
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
        };

        let path_via = path_names
            .split(" -> ")
            .map(|s| s.to_string())
            .collect::<Vec<_>>();

        Ok(ImpactItem {
            symbol: sym,
            file_path,
            depth: depth as usize,
            path_via,
        })
    })?;

    let mut seen_symbols = std::collections::HashSet::new();
    let mut affected_symbols = Vec::new();
    let mut affected_files_set = std::collections::BTreeSet::new();

    for r in rows {
        let item = r?;
        if let Some(sym_id) = item.symbol.id {
            if seen_symbols.insert(sym_id) {
                affected_files_set.insert(item.file_path.clone());
                affected_symbols.push(item);
            }
        }
    }

    let affected_files = affected_files_set.into_iter().collect::<Vec<_>>();
    let total_affected = affected_symbols.len();

    Ok(ImpactResult {
        root_symbol,
        affected_symbols,
        affected_files,
        total_affected,
    })
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/query.rs"]
mod tests;
