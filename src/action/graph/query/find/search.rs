use rusqlite::params;

use crate::action::graph::query::types::{QueryError, SymbolLocation, map_symbol_and_path_row};
use crate::store::graph::db::GraphDb;

/// Matches `s.name` starting with any of the casings bound to `?1`..`?4`, as
/// index range seeks rather than a `LIKE` scan.
pub(super) const NAME_PREFIX_MATCH: &str = "((s.name >= ?1 AND s.name < ?1 || char(1114111))
        OR (s.name >= ?2 AND s.name < ?2 || char(1114111))
        OR (s.name >= ?3 AND s.name < ?3 || char(1114111))
        OR (s.name >= ?4 AND s.name < ?4 || char(1114111)))";

/// The casings a prefix search tries: as typed, lowercase, first letter upper, and UPPER.
pub(super) fn prefix_casings(term: &str) -> [String; 4] {
    let mut chars = term.chars();
    let first_upper = chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    let mut casings = vec![term.to_owned()];
    for casing in [term.to_lowercase(), first_upper, term.to_uppercase()] {
        if !casings.contains(&casing) {
            casings.push(casing);
        }
    }
    std::array::from_fn(|i| casings.get(i).map_or_else(|| term.to_owned(), Clone::clone))
}

/// Records one candidate row if its symbol id has not been seen yet.
/// Returns whether `results` has now reached `limit`.
fn push_unique(
    results: &mut Vec<SymbolLocation>,
    seen_ids: &mut std::collections::HashSet<i64>,
    row: (crate::domain::graph::Symbol, String),
    limit: usize,
) -> bool {
    let (symbol, file_path) = row;
    if let Some(id) = symbol.id
        && seen_ids.insert(id)
    {
        results.push(SymbolLocation { symbol, file_path });
    }
    results.len() >= limit
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
                    s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path
             FROM visible_symbols s
             JOIN visible_files f ON s.file_id = f.id
             WHERE s.name = ?1 AND (?2 IS NULL OR s.repo = ?2)
             LIMIT ?3",
        )?;
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = stmt.query_map(params![query, repo, limit_i64], map_symbol_and_path_row)?;
        for row in rows {
            if push_unique(&mut results, &mut seen_ids, row?, limit) {
                return Ok(results);
            }
        }
    }

    // 2. Prefix matches
    if results.len() < limit {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                    s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path
             FROM visible_symbols s
             JOIN visible_files f ON s.file_id = f.id
             WHERE {NAME_PREFIX_MATCH} AND (?5 IS NULL OR s.repo = ?5)
             ORDER BY length(s.name) ASC, s.id ASC
             LIMIT ?6",
        ))?;
        let [c1, c2, c3, c4] = prefix_casings(query);
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = stmt.query_map(
            params![c1, c2, c3, c4, repo, limit_i64],
            map_symbol_and_path_row,
        )?;
        for row in rows {
            if push_unique(&mut results, &mut seen_ids, row?, limit) {
                return Ok(results);
            }
        }
    }

    // 3. FTS5 search
    if results.len() < limit {
        let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
        if let Ok(mut stmt) = conn.prepare_cached(
            "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                    s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path
             FROM symbols_fts fts
             JOIN visible_symbols s ON fts.rowid = s.id
             JOIN visible_files f ON s.file_id = f.id
             WHERE symbols_fts MATCH ?1 AND (?2 IS NULL OR s.repo = ?2)
             ORDER BY rank
             LIMIT ?3",
        ) && let Ok(rows) = stmt.query_map(
            params![fts_query, repo, i64::try_from(limit).unwrap_or(i64::MAX)],
            map_symbol_and_path_row,
        ) {
            for row in rows.flatten() {
                if push_unique(&mut results, &mut seen_ids, row, limit) {
                    return Ok(results);
                }
            }
        }
    }

    Ok(results)
}
