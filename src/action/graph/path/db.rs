//! Database edge retrieval and candidate resolution for graph pathfinding.

use std::collections::HashSet;

use rusqlite::params;

use super::PathError;
use crate::domain::graph::EdgeKind;
use crate::store::graph::db::parse_edge_kind;

#[derive(Debug, Clone)]
pub(super) struct EdgeRecord {
    pub from_symbol_id: Option<i64>,
    pub to_symbol_id: Option<i64>,
    pub from_name: String,
    pub to_name: String,
    pub kind: EdgeKind,
    pub line: usize,
}

pub(super) fn resolve_candidates(
    conn: &rusqlite::Connection,
    name_or_file: &str,
) -> Result<HashSet<i64>, PathError> {
    let mut candidates = HashSet::new();

    // Try matching symbol name directly
    let mut sym_stmt = conn.prepare_cached("SELECT id FROM visible_symbols WHERE name = ?1")?;
    let mut rows = sym_stmt.query(params![name_or_file])?;
    while let Some(row) = rows.next()? {
        candidates.insert(row.get(0)?);
    }

    // Try matching file path or suffix
    if candidates.is_empty() {
        let normalized = name_or_file.replace('\\', "/");
        let like_pattern = format!("%/{}", normalized);
        let mut file_stmt = conn.prepare_cached(
            "SELECT s.id FROM visible_symbols s
             JOIN visible_files f ON s.file_id = f.id
             WHERE f.path = ?1 OR (f.repo || '/' || f.path) = ?1 OR f.path LIKE ?2",
        )?;
        let mut rows = file_stmt.query(params![normalized, like_pattern])?;
        while let Some(row) = rows.next()? {
            candidates.insert(row.get(0)?);
        }
    }

    Ok(candidates)
}

pub(super) fn get_outgoing_edges(
    conn: &rusqlite::Connection,
    symbol_id: i64,
) -> Result<Vec<EdgeRecord>, PathError> {
    let mut stmt = conn.prepare_cached(
        "SELECT
            e.from_symbol_id,
            COALESCE(e.to_symbol_id, s_to.id) AS resolved_to_id,
            s_from.name AS from_name,
            COALESCE(s_to.name, e.to_name) AS to_name,
            e.kind,
            e.line
         FROM visible_edges e
         JOIN visible_symbols s_from ON e.from_symbol_id = s_from.id
         LEFT JOIN visible_symbols s_to ON (e.to_symbol_id = s_to.id OR (e.to_symbol_id IS NULL AND e.to_name = s_to.name))
         WHERE e.from_symbol_id = ?1",
    )?;

    let mut records = Vec::new();
    let mut rows = stmt.query(params![symbol_id])?;
    while let Some(row) = rows.next()? {
        let from_sym: Option<i64> = row.get(0)?;
        let to_sym: Option<i64> = row.get(1)?;
        let from_name: String = row.get(2)?;
        let to_name: Option<String> = row.get(3)?;
        let kind_raw: String = row.get(4)?;
        let line: i64 = row.get(5)?;

        if let Some(target_name) = to_name {
            records.push(EdgeRecord {
                from_symbol_id: from_sym,
                to_symbol_id: to_sym,
                from_name,
                to_name: target_name,
                kind: parse_edge_kind(&kind_raw),
                line: line as usize,
            });
        }
    }

    Ok(records)
}

pub(super) fn get_incoming_edges(
    conn: &rusqlite::Connection,
    symbol_id: i64,
) -> Result<Vec<EdgeRecord>, PathError> {
    let mut stmt = conn.prepare_cached(
        "SELECT
            e.from_symbol_id,
            COALESCE(e.to_symbol_id, ?1) AS resolved_to_id,
            s_from.name AS from_name,
            COALESCE(s_to.name, e.to_name) AS to_name,
            e.kind,
            e.line
         FROM visible_edges e
         JOIN visible_symbols s_from ON e.from_symbol_id = s_from.id
         LEFT JOIN visible_symbols s_to ON s_to.id = ?1
         WHERE (e.to_symbol_id = ?1 OR (e.to_symbol_id IS NULL AND e.to_name = (SELECT name FROM visible_symbols WHERE id = ?1)))",
    )?;

    let mut records = Vec::new();
    let mut rows = stmt.query(params![symbol_id])?;
    while let Some(row) = rows.next()? {
        let from_sym: Option<i64> = row.get(0)?;
        let to_sym: Option<i64> = row.get(1)?;
        let from_name: String = row.get(2)?;
        let to_name: Option<String> = row.get(3)?;
        let kind_raw: String = row.get(4)?;
        let line: i64 = row.get(5)?;

        if let Some(target_name) = to_name {
            records.push(EdgeRecord {
                from_symbol_id: from_sym,
                to_symbol_id: to_sym,
                from_name,
                to_name: target_name,
                kind: parse_edge_kind(&kind_raw),
                line: line as usize,
            });
        }
    }

    Ok(records)
}
