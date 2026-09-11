//! Transitive blast-radius impact analysis.

use rusqlite::{OptionalExtension, params};

use super::types::{ImpactItem, ImpactResult, QueryError, map_symbol_row};
use crate::domain::graph::{Span, Symbol};
use crate::store::graph::db::{GraphDb, parse_symbol_kind};

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
            )
            JOIN visible_symbols s ON e.from_symbol_id = s.id
            WHERE cg.depth < ?2
              AND e.from_symbol_id IS NOT NULL
              AND instr(cg.visited_ids, ',' || CAST(s.id AS TEXT) || ',') = 0
        )
        SELECT cg.symbol_id, cg.depth, cg.path_names,
               s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
               s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity,
               f.path
        FROM caller_graph cg
        JOIN visible_symbols s ON cg.symbol_id = s.id
        JOIN visible_files f ON s.file_id = f.id
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
        let complexity = row.get::<_, Option<i64>>(16)?.map(|c| c as u32);
        let file_path: String = row.get(17)?;
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
            complexity,
        };

        let path_via = path_names
            .split(" -> ")
            .map(|s| s.to_owned())
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
