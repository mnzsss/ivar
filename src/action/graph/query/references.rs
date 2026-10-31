use rusqlite::params;

use super::types::{QueryError, ReferenceSite};
use crate::store::graph::db::GraphDb;

/// Lists references to a symbol that sit outside any symbol body, such as imports.
pub fn get_references(
    db: &GraphDb,
    symbol_name: &str,
    repo: Option<&str>,
) -> Result<Vec<ReferenceSite>, QueryError> {
    let conn = db.conn();
    // The unary `+` keeps SQLite off `idx_edges_from_symbol`. Most edges in a hall
    // have no source symbol, so that index matches over a million rows; the target
    // indexes return the same rows in under a millisecond.
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT f.repo, f.path, e.line
         FROM visible_edges e
         JOIN visible_files f ON e.file_id = f.id
         WHERE +e.from_symbol_id IS NULL
           AND e.kind IN ('REFERENCES', 'references')
           AND (
               e.to_symbol_id IN (
                   SELECT id FROM visible_symbols WHERE name = ?1 AND (?2 IS NULL OR repo = ?2)
               )
               OR (e.to_symbol_id IS NULL AND e.to_name = ?1 AND (?2 IS NULL OR e.repo = ?2))
               OR (e.to_symbol_id IN (SELECT id FROM hidden_symbols) AND e.to_name = ?1 AND (?2 IS NULL OR e.repo = ?2))
           )
         ORDER BY f.repo, f.path, e.line",
    )?;
    let sites = stmt
        .query_map(params![symbol_name, repo], map_reference_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(sites)
}

/// Lists references to one definition that sit outside any symbol body, such as imports.
pub fn get_references_of(db: &GraphDb, symbol_id: i64) -> Result<Vec<ReferenceSite>, QueryError> {
    let mut stmt = db.conn().prepare_cached(
        "SELECT DISTINCT f.repo, f.path, e.line
         FROM visible_edges e
         JOIN visible_files f ON e.file_id = f.id
         WHERE +e.from_symbol_id IS NULL
           AND e.kind IN ('REFERENCES', 'references')
           AND (
               e.to_symbol_id = ?1
               OR (e.to_symbol_id IS NULL AND e.to_name = (SELECT name FROM visible_symbols WHERE id = ?1))
               OR (e.to_symbol_id IN (SELECT id FROM hidden_symbols) AND e.to_name = (SELECT name FROM visible_symbols WHERE id = ?1))
           )
         ORDER BY f.repo, f.path, e.line",
    )?;
    let sites = stmt
        .query_map(params![symbol_id], map_reference_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(sites)
}

fn map_reference_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReferenceSite> {
    Ok(ReferenceSite {
        repo: row.get(0)?,
        file_path: row.get(1)?,
        line: usize::try_from(row.get::<_, i64>(2)?).unwrap_or(usize::MAX),
    })
}
