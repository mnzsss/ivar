//! Symbol search implementation: exact match, prefix match, and FTS5 ranking.

use rusqlite::params;

use super::types::{QueryError, SymbolLocation, map_symbol_and_path_row};
use crate::store::graph::db::GraphDb;

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
            if let Some(id) = sym.id
                && seen_ids.insert(id)
            {
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
        let rows = stmt.query_map(
            params![prefix_query, repo, limit as i64],
            map_symbol_and_path_row,
        )?;
        for row in rows {
            let (sym, path) = row?;
            if let Some(id) = sym.id
                && seen_ids.insert(id)
            {
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
        ) && let Ok(rows) = stmt.query_map(
            params![fts_query, repo, limit as i64],
            map_symbol_and_path_row,
        ) {
            for row in rows.flatten() {
                let (sym, path) = row;
                if let Some(id) = sym.id
                    && seen_ids.insert(id)
                {
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

    Ok(results)
}
