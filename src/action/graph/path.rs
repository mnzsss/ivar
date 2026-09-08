//! Bidirectional BFS shortest-path finder between symbols or files.
//!
//! Synchronously traverses graph edges in both directions (forward from source,
//! backward from destination) until frontiers intersect or `max_hops` is reached.

use std::collections::{HashMap, HashSet, VecDeque};

use rusqlite::params;
use thiserror::Error;

use crate::domain::graph::{EdgeKind, PathResult, PathStep};
use crate::store::graph::db::{GraphDb, parse_edge_kind};

/// Error returned during shortest path traversal.
#[derive(Debug, Error)]
pub enum PathError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("start node not found: {0}")]
    StartNotFound(String),
    #[error("target node not found: {0}")]
    TargetNotFound(String),
    #[error("invalid max_hops: {0} (must be > 0)")]
    InvalidMaxHops(usize),
}

#[derive(Debug, Clone)]
struct EdgeRecord {
    from_symbol_id: Option<i64>,
    to_symbol_id: Option<i64>,
    from_name: String,
    to_name: String,
    kind: EdgeKind,
    line: usize,
}

/// Finds the shortest path between two symbols or files using bidirectional BFS.
///
/// Returns `Some(PathResult)` if a connecting sequence of directed edges is found
/// within `max_hops`, or `None` if no path exists.
pub fn find_shortest_path(
    db: &GraphDb,
    from_symbol_or_file: &str,
    to_symbol_or_file: &str,
    max_hops: usize,
) -> Result<Option<PathResult>, PathError> {
    if max_hops == 0 {
        return Err(PathError::InvalidMaxHops(0));
    }

    let conn = db.conn();

    // 1. Identify start node candidate symbol IDs / names
    let start_symbols = resolve_candidates(conn, from_symbol_or_file)?;
    if start_symbols.is_empty() {
        return Err(PathError::StartNotFound(from_symbol_or_file.to_owned()));
    }

    // 2. Identify target node candidate symbol IDs / names
    let target_symbols = resolve_candidates(conn, to_symbol_or_file)?;
    if target_symbols.is_empty() {
        return Err(PathError::TargetNotFound(to_symbol_or_file.to_owned()));
    }

    // Check if start and target already overlap
    for start_id in &start_symbols {
        if target_symbols.contains(start_id) {
            return Ok(Some(PathResult {
                from: from_symbol_or_file.to_owned(),
                to: to_symbol_or_file.to_owned(),
                steps: Vec::new(),
            }));
        }
    }

    // Bidirectional BFS on symbol IDs
    // Forward search: symbol_id -> (predecessor_symbol_id, PathStep)
    let mut forward_visited: HashMap<i64, Option<(i64, PathStep)>> = HashMap::new();
    let mut forward_queue: VecDeque<(i64, usize)> = VecDeque::new();

    for &s in &start_symbols {
        forward_visited.insert(s, None);
        forward_queue.push_back((s, 0));
    }

    // Backward search: symbol_id -> (successor_symbol_id, PathStep)
    let mut backward_visited: HashMap<i64, Option<(i64, PathStep)>> = HashMap::new();
    let mut backward_queue: VecDeque<(i64, usize)> = VecDeque::new();

    for &t in &target_symbols {
        backward_visited.insert(t, None);
        backward_queue.push_back((t, 0));
    }

    let mut meeting_node: Option<i64> = None;

    while !forward_queue.is_empty() && !backward_queue.is_empty() {
        // Expand forward frontier
        if let Some((curr, depth)) = forward_queue.pop_front()
            && depth < max_hops
        {
            // Find outgoing edges from `curr`
            let outgoing = get_outgoing_edges(conn, curr)?;
            for edge in outgoing {
                if let Some(next_id) = edge.to_symbol_id {
                    let step = PathStep {
                        source: edge.from_name,
                        target: edge.to_name,
                        edge_kind: edge.kind,
                        line: edge.line,
                    };

                    if let std::collections::hash_map::Entry::Vacant(e) =
                        forward_visited.entry(next_id)
                    {
                        e.insert(Some((curr, step.clone())));
                        forward_queue.push_back((next_id, depth + 1));
                    }

                    if backward_visited.contains_key(&next_id) {
                        meeting_node = Some(next_id);
                        break;
                    }
                }
            }
        }

        if meeting_node.is_some() {
            break;
        }

        // Expand backward frontier
        if let Some((curr, depth)) = backward_queue.pop_front()
            && depth < max_hops
        {
            // Find incoming edges to `curr`
            let incoming = get_incoming_edges(conn, curr)?;
            for edge in incoming {
                if let Some(prev_id) = edge.from_symbol_id {
                    let step = PathStep {
                        source: edge.from_name,
                        target: edge.to_name,
                        edge_kind: edge.kind,
                        line: edge.line,
                    };

                    if let std::collections::hash_map::Entry::Vacant(e) =
                        backward_visited.entry(prev_id)
                    {
                        e.insert(Some((curr, step.clone())));
                        backward_queue.push_back((prev_id, depth + 1));
                    }

                    if forward_visited.contains_key(&prev_id) {
                        meeting_node = Some(prev_id);
                        break;
                    }
                }
            }
        }

        if meeting_node.is_some() {
            break;
        }
    }

    let meeting_node = match meeting_node {
        Some(node) => node,
        None => return Ok(None),
    };

    // Reconstruct full path
    let mut steps = Vec::new();

    // 1. Trace back from meeting_node to start in forward_visited
    let mut curr = meeting_node;
    let mut forward_steps = Vec::new();
    while let Some(Some((pred, step))) = forward_visited.get(&curr) {
        forward_steps.push(step.clone());
        curr = *pred;
    }
    forward_steps.reverse();
    steps.extend(forward_steps);

    // 2. Trace forward from meeting_node to target in backward_visited
    let mut curr = meeting_node;
    while let Some(Some((succ, step))) = backward_visited.get(&curr) {
        steps.push(step.clone());
        curr = *succ;
    }

    if steps.len() > max_hops {
        return Ok(None);
    }

    Ok(Some(PathResult {
        from: from_symbol_or_file.to_owned(),
        to: to_symbol_or_file.to_owned(),
        steps,
    }))
}

fn resolve_candidates(
    conn: &rusqlite::Connection,
    name_or_file: &str,
) -> Result<HashSet<i64>, PathError> {
    let mut candidates = HashSet::new();

    // Try matching symbol name directly
    let mut sym_stmt = conn.prepare_cached("SELECT id FROM symbols WHERE name = ?1")?;
    let mut rows = sym_stmt.query(params![name_or_file])?;
    while let Some(row) = rows.next()? {
        candidates.insert(row.get(0)?);
    }

    // Try matching file path or suffix
    if candidates.is_empty() {
        let normalized = name_or_file.replace('\\', "/");
        let like_pattern = format!("%/{}", normalized);
        let mut file_stmt = conn.prepare_cached(
            "SELECT s.id FROM symbols s
             JOIN files f ON s.file_id = f.id
             WHERE f.path = ?1 OR f.path LIKE ?2",
        )?;
        let mut rows = file_stmt.query(params![normalized, like_pattern])?;
        while let Some(row) = rows.next()? {
            candidates.insert(row.get(0)?);
        }
    }

    Ok(candidates)
}

fn get_outgoing_edges(
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
         FROM edges e
         JOIN symbols s_from ON e.from_symbol_id = s_from.id
         LEFT JOIN symbols s_to ON (e.to_symbol_id = s_to.id OR (e.to_symbol_id IS NULL AND e.to_name = s_to.name))
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

fn get_incoming_edges(
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
         FROM edges e
         JOIN symbols s_from ON e.from_symbol_id = s_from.id
         LEFT JOIN symbols s_to ON s_to.id = ?1
         WHERE (e.to_symbol_id = ?1 OR (e.to_symbol_id IS NULL AND e.to_name = (SELECT name FROM symbols WHERE id = ?1)))",
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

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/path.rs"]
mod tests;
