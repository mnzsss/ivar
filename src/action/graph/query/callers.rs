use rusqlite::params;

use super::name_match::{generic_names_sql, language_sql};
use super::types::{CallerInfo, QueryError, map_symbol_and_path_row};
use crate::domain::graph::Provenance;
use crate::store::graph::db::{GraphDb, parse_edge_kind, parse_provenance};

/// Retrieves all callers referencing the given symbol name.
///
/// Callers resolved to a definition with that name come first. Edges that only
/// name the target are kept when they share a language with a definition, are
/// not generic names, and prefer the target's repo; they are reported as
/// inferred.
pub fn get_callers(
    db: &GraphDb,
    symbol_name: &str,
    repo: Option<&str>,
    cross_repo: bool,
    min_confidence: f64,
) -> Result<Vec<CallerInfo>, QueryError> {
    let conn = db.conn();
    let target_lang = language_sql("tf.path");
    let caller_lang = language_sql("f.path");
    let generic = generic_names_sql();
    let sql = format!(
        "WITH targets AS (
             SELECT t.id, t.repo, {target_lang} AS lang
             FROM visible_symbols t
             JOIN visible_files tf ON t.file_id = tf.id
             WHERE t.name = ?1 AND (?2 IS NULL OR t.repo = ?2)
         )
         SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path,
                e.kind, e.provenance, e.confidence, e.line, e.col,
                (e.to_symbol_id IS NULL OR e.to_symbol_id NOT IN (SELECT id FROM targets)) AS name_only
         FROM visible_edges e
         JOIN visible_symbols s ON e.from_symbol_id = s.id
         JOIN visible_files f ON s.file_id = f.id
         WHERE (
             e.to_symbol_id IN (SELECT id FROM targets)
             OR (
                 (e.to_symbol_id IS NULL OR e.to_symbol_id IN (SELECT id FROM hidden_symbols))
                 AND e.to_name = ?1 AND (?2 IS NULL OR e.repo = ?2)
                 AND ?1 NOT IN ({generic})
                 AND (
                     NOT EXISTS (SELECT 1 FROM targets)
                     OR EXISTS (SELECT 1 FROM targets WHERE lang = {caller_lang})
                 )
             )
         )
         AND e.confidence >= ?3
         AND (?4 = 1 OR ?2 IS NULL OR s.repo = ?2)
         ORDER BY name_only ASC,
                  (e.repo IN (SELECT repo FROM targets)) DESC,
                  e.confidence DESC, s.name ASC"
    );
    let mut stmt = conn.prepare_cached(&sql)?;

    let cross_repo_int = if cross_repo { 1 } else { 0 };
    let rows = stmt.query_map(
        params![symbol_name, repo, min_confidence, cross_repo_int],
        map_caller_row,
    )?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Retrieves the callers of one definition, plus unresolved edges naming it.
///
/// Edges resolved to the definition come first. Edges that only name it must
/// share its language and a non-generic name; they are reported as inferred and
/// ranked after resolved ones, same-repo before other repos.
pub fn get_callers_of(
    db: &GraphDb,
    symbol_id: i64,
    min_confidence: f64,
) -> Result<Vec<CallerInfo>, QueryError> {
    let target_lang = language_sql("tf.path");
    let caller_lang = language_sql("f.path");
    let generic = generic_names_sql();
    // The id prefilter keeps SQLite from walking every unresolved edge to evaluate the OR below;
    // that predicate remains the filter.
    let sql = format!(
        "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path,
                e.kind, e.provenance, e.confidence, e.line, e.col,
                (e.to_symbol_id IS NOT ?1) AS name_only
         FROM visible_edges e
         JOIN visible_symbols s ON e.from_symbol_id = s.id
         JOIN visible_files f ON s.file_id = f.id
         LEFT JOIN visible_symbols t ON t.id = ?1
         LEFT JOIN visible_files tf ON t.file_id = tf.id
         WHERE e.id IN (
             SELECT by_target.id FROM visible_edges by_target WHERE by_target.to_symbol_id = ?1
             UNION
             SELECT by_name.id
             FROM visible_symbols named
             JOIN visible_edges by_name ON by_name.to_name = named.name
             WHERE named.id = ?1
               AND (by_name.to_symbol_id IS NULL OR by_name.to_symbol_id IN (SELECT id FROM hidden_symbols))
         )
         AND (
             e.to_symbol_id = ?1
             OR (
                 (e.to_symbol_id IS NULL OR e.to_symbol_id IN (SELECT id FROM hidden_symbols))
                 AND e.to_name = t.name
                 AND e.to_name NOT IN ({generic})
                 AND {target_lang} = {caller_lang}
             )
         )
         AND e.confidence >= ?2
         ORDER BY name_only ASC, (e.repo = t.repo) DESC, e.confidence DESC, s.name ASC"
    );
    let mut stmt = db.conn().prepare_cached(&sql)?;
    let rows = stmt.query_map(params![symbol_id, min_confidence], map_caller_row)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn map_caller_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CallerInfo> {
    let (caller, caller_file_path) = map_symbol_and_path_row(row)?;
    let kind_raw: String = row.get(15)?;
    let provenance_raw: String = row.get(16)?;
    let name_only: bool = row.get(20)?;
    let mut provenance = parse_provenance(&provenance_raw);
    if name_only && provenance == Provenance::Extracted {
        provenance = Provenance::Inferred;
    }
    Ok(CallerInfo {
        caller,
        caller_file_path,
        edge_kind: parse_edge_kind(&kind_raw),
        provenance,
        confidence: row.get(17)?,
        line: usize::try_from(row.get::<_, i64>(18)?).unwrap_or(usize::MAX),
        col: usize::try_from(row.get::<_, i64>(19)?).unwrap_or(usize::MAX),
    })
}
