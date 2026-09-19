//! Transitive blast-radius impact analysis.

use rusqlite::{OptionalExtension, params};

use super::types::{ImpactItem, ImpactResult, QueryError, map_symbol_row};
use crate::store::graph::db::GraphDb;
use crate::store::graph::db::row::symbol_from_row;

/// Analyzes blast-radius impact of changing a symbol by recursively traversing callers.
pub fn get_impact(
    db: &GraphDb,
    symbol_id: i64,
    max_depth: usize,
) -> Result<ImpactResult, QueryError> {
    let conn = db.conn();
    let mut root_stmt = conn.prepare_cached(
        "SELECT id, file_id, repo, name, kind, scope, signature, docstring,
                start_line, start_col, end_line, end_col, is_exported, complexity
         FROM visible_symbols
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
            -- Base case: the initial symbol
            SELECT s.id, s.name, 0, ',' || CAST(s.id AS TEXT) || ',', s.name
            FROM visible_symbols s
            WHERE s.id = ?1

            UNION ALL

            -- Recursive step: callers of the current symbol
            SELECT
                e.from_symbol_id,
                s.name,
                cg.depth + 1,
                cg.visited_ids || CAST(s.id AS TEXT) || ',',
                cg.path_names || ' -> ' || s.name
            FROM visible_edges e
            JOIN caller_graph cg ON (
                e.to_symbol_id = cg.symbol_id
                OR (e.to_symbol_id IS NULL AND e.to_name = cg.symbol_name)
                OR (e.to_symbol_id IN (SELECT id FROM hidden_symbols) AND e.to_name = cg.symbol_name)
            )
            JOIN visible_symbols s ON e.from_symbol_id = s.id
            WHERE cg.depth < ?2
              AND e.from_symbol_id IS NOT NULL
              AND instr(cg.visited_ids, ',' || CAST(s.id AS TEXT) || ',') = 0
        )
        SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
               s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity,
               cg.depth, cg.path_names, f.path
        FROM caller_graph cg
        CROSS JOIN visible_symbols s ON cg.symbol_id = s.id
        JOIN visible_files f ON s.file_id = f.id
        WHERE cg.symbol_id != ?1
        ORDER BY cg.depth ASC, s.name ASC",
    )?;

    let max_depth_i64 = i64::try_from(max_depth).unwrap_or(i64::MAX);
    let rows = stmt.query_map(params![symbol_id, max_depth_i64], |row| {
        let sym = symbol_from_row(row)?;
        let depth: i64 = row.get(14)?;
        let path_names: String = row.get(15)?;
        let file_path: String = row.get(16)?;

        let path_via = path_names
            .split(" -> ")
            .map(|s| s.to_owned())
            .collect::<Vec<_>>();

        Ok(ImpactItem {
            symbol: sym,
            file_path,
            depth: usize::try_from(depth).unwrap_or(usize::MAX),
            path_via,
        })
    })?;

    let mut seen_symbols = std::collections::HashSet::new();
    let mut affected_symbols = Vec::new();
    let mut affected_files_set = std::collections::BTreeSet::new();

    for r in rows {
        let item = r?;
        if let Some(sym_id) = item.symbol.id
            && seen_symbols.insert(sym_id)
        {
            affected_files_set.insert(item.file_path.clone());
            affected_symbols.push(item);
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
