use rusqlite::{Connection, params};
use std::collections::{HashSet, VecDeque};

use crate::action::graph::path::{PathResult, find_shortest_path};
use crate::action::graph::query::{
    find_symbols, get_callees, get_callers, get_impact,
    types::{ImpactResult, SymbolLocation},
};
use crate::domain::graph::Span;
use crate::store::graph::db::{GraphDb, parse_edge_kind, parse_provenance, parse_symbol_kind};

use super::types::*;

pub fn collect_subgraph(
    db: &GraphDb,
    seed: &ViewSeed,
    depth: usize,
    limit: usize,
) -> Result<ViewerGraph, ViewError> {
    if depth == 0 || depth > MAX_DEPTH {
        return Err(ViewError::InvalidParam(format!(
            "depth must be between 1 and {MAX_DEPTH}"
        )));
    }
    if limit == 0 || limit > MAX_NODES {
        return Err(ViewError::InvalidParam(format!(
            "limit must be between 1 and {MAX_NODES}"
        )));
    }

    let conn = db.conn();
    let seed_limit = match seed {
        ViewSeed::Default | ViewSeed::Repo(_) => {
            if depth > 0 {
                (limit / 4).clamp(10, 80).min(limit)
            } else {
                limit
            }
        }
        _ => limit,
    };
    let seed_nodes = resolve_seed_nodes(conn, seed, seed_limit)?;
    if seed_nodes.is_empty() {
        return Ok(ViewerGraph {
            nodes: Vec::new(),
            edges: Vec::new(),
            truncated: false,
            depth,
        });
    }

    let mut visited_node_ids = HashSet::new();
    let mut ordered_nodes = Vec::new();
    let mut queue = VecDeque::new();
    let mut truncated = false;

    for node in seed_nodes {
        if visited_node_ids.insert(node.id) {
            queue.push_back((node.id, 0));
            ordered_nodes.push(node);
            if ordered_nodes.len() >= limit {
                truncated = true;
                break;
            }
        }
    }

    while let Some((node_id, curr_depth)) = queue.pop_front() {
        if curr_depth >= depth || ordered_nodes.len() >= limit {
            if ordered_nodes.len() >= limit {
                truncated = true;
            }
            continue;
        }

        let neighbors = fetch_neighbor_nodes(conn, node_id)?;
        for neighbor in neighbors {
            if visited_node_ids.insert(neighbor.id) {
                if ordered_nodes.len() < limit {
                    queue.push_back((neighbor.id, curr_depth + 1));
                    ordered_nodes.push(neighbor);
                } else {
                    truncated = true;
                    break;
                }
            }
        }
    }

    let node_id_set: HashSet<i64> = ordered_nodes.iter().map(|n| n.id).collect();
    let edges = fetch_connecting_edges(conn, &node_id_set)?;

    Ok(ViewerGraph {
        nodes: ordered_nodes,
        edges,
        truncated,
        depth,
    })
}

pub fn expand_node(db: &GraphDb, symbol_id: i64, limit: usize) -> Result<ViewerGraph, ViewError> {
    let cap = limit.min(MAX_NODES);
    collect_subgraph(db, &ViewSeed::Symbol(format!("id:{symbol_id}")), 1, cap)
}

pub fn get_node_details(db: &GraphDb, symbol_id: i64) -> Result<ViewerNodeDetails, ViewError> {
    let node = fetch_single_node(db.conn(), symbol_id)?
        .ok_or_else(|| ViewError::SeedNotFound(format!("symbol id {symbol_id}")))?;
    let callers = get_callers(db, &node.name, Some(&node.repo), true, 0.0)
        .map_err(|e| ViewError::Query(e.to_string()))?;
    let callees = get_callees(db, symbol_id).map_err(|e| ViewError::Query(e.to_string()))?;
    Ok(ViewerNodeDetails {
        node,
        callers,
        callees,
    })
}

pub fn search_symbols(
    db: &GraphDb,
    query: &str,
    repo: Option<&str>,
    limit: usize,
) -> Result<Vec<SymbolLocation>, ViewError> {
    find_symbols(db, query, repo, limit.min(MAX_NODES)).map_err(|e| ViewError::Query(e.to_string()))
}

pub fn query_path(
    db: &GraphDb,
    from: &str,
    to: &str,
    max_hops: usize,
) -> Result<Option<PathResult>, ViewError> {
    find_shortest_path(db, from, to, max_hops).map_err(|e| ViewError::Query(e.to_string()))
}

pub fn query_impact(
    db: &GraphDb,
    symbol: &str,
    repo: Option<&str>,
    max_depth: usize,
) -> Result<ImpactResult, ViewError> {
    // Resolve symbol id first
    let conn = db.conn();
    let mut stmt = conn.prepare_cached(
        "SELECT id FROM symbols WHERE name = ?1 AND (?2 IS NULL OR repo = ?2) ORDER BY is_exported DESC, id ASC LIMIT 1",
    )?;
    let symbol_id: i64 = stmt
        .query_row(params![symbol, repo], |r| r.get(0))
        .map_err(|_| ViewError::SeedNotFound(symbol.to_owned()))?;

    get_impact(db, symbol_id, max_depth.min(MAX_DEPTH)).map_err(|e| ViewError::Query(e.to_string()))
}

fn map_node_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ViewerNode> {
    let id: i64 = row.get(0)?;
    let repo: String = row.get(1)?;
    let name: String = row.get(2)?;
    let kind_str: String = row.get(3)?;
    let signature: Option<String> = row.get(4)?;
    let start_line: usize = row.get(5)?;
    let start_col: usize = row.get(6)?;
    let end_line: usize = row.get(7)?;
    let end_col: usize = row.get(8)?;
    let exported_int: i64 = row.get(9)?;
    let complexity: Option<u32> = row.get(10)?;
    let file: String = row.get(11)?;

    Ok(ViewerNode {
        id,
        repo,
        name,
        kind: parse_symbol_kind(&kind_str),
        signature,
        file,
        span: Span::new(start_line, start_col, end_line, end_col),
        exported: exported_int != 0,
        complexity,
    })
}

fn fetch_single_node(conn: &Connection, symbol_id: i64) -> Result<Option<ViewerNode>, ViewError> {
    let mut stmt = conn.prepare_cached(
        "SELECT s.id, s.repo, s.name, s.kind, s.signature,
                s.start_line, s.start_col, s.end_line, s.end_col,
                s.is_exported, s.complexity, f.path
         FROM symbols s
         JOIN files f ON s.file_id = f.id
         WHERE s.id = ?1",
    )?;
    let mut rows = stmt.query(params![symbol_id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(map_node_row(row)?))
    } else {
        Ok(None)
    }
}

fn resolve_seed_nodes(
    conn: &Connection,
    seed: &ViewSeed,
    limit: usize,
) -> Result<Vec<ViewerNode>, ViewError> {
    match seed {
        ViewSeed::Default => {
            let mut stmt = conn.prepare_cached(
                "SELECT s.id, s.repo, s.name, s.kind, s.signature,
                        s.start_line, s.start_col, s.end_line, s.end_col,
                        s.is_exported, s.complexity, f.path
                 FROM symbols s
                 JOIN files f ON s.file_id = f.id
                 LEFT JOIN edges e ON (e.from_symbol_id = s.id OR e.to_symbol_id = s.id)
                 GROUP BY s.id
                 ORDER BY count(e.id) DESC, s.is_exported DESC, s.id ASC
                 LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit as i64], map_node_row)?;
            let mut nodes = Vec::new();
            for r in rows {
                nodes.push(r?);
            }
            Ok(nodes)
        }
        ViewSeed::Repo(repo) => {
            let mut stmt = conn.prepare_cached(
                "SELECT s.id, s.repo, s.name, s.kind, s.signature,
                        s.start_line, s.start_col, s.end_line, s.end_col,
                        s.is_exported, s.complexity, f.path
                 FROM symbols s
                 JOIN files f ON s.file_id = f.id
                 LEFT JOIN edges e ON (e.from_symbol_id = s.id OR e.to_symbol_id = s.id)
                 WHERE s.repo = ?1
                 GROUP BY s.id
                 ORDER BY count(e.id) DESC, s.is_exported DESC, s.id ASC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![repo, limit as i64], map_node_row)?;
            let mut nodes = Vec::new();
            for r in rows {
                nodes.push(r?);
            }
            Ok(nodes)
        }
        ViewSeed::File(path) => {
            let mut stmt = conn.prepare_cached(
                "SELECT s.id, s.repo, s.name, s.kind, s.signature,
                        s.start_line, s.start_col, s.end_line, s.end_col,
                        s.is_exported, s.complexity, f.path
                 FROM symbols s
                 JOIN files f ON s.file_id = f.id
                 WHERE f.path = ?1 OR f.path LIKE ?2
                 ORDER BY s.is_exported DESC, s.id ASC
                 LIMIT ?3",
            )?;
            let like_pattern = format!("%/{path}");
            let rows = stmt.query_map(params![path, like_pattern, limit as i64], map_node_row)?;
            let mut nodes = Vec::new();
            for r in rows {
                nodes.push(r?);
            }
            Ok(nodes)
        }
        ViewSeed::Symbol(sym) => {
            if let Some(id_str) = sym.strip_prefix("id:")
                && let Ok(id) = id_str.parse::<i64>()
            {
                if let Some(node) = fetch_single_node(conn, id)? {
                    return Ok(vec![node]);
                }
                return Ok(Vec::new());
            }

            let mut stmt = conn.prepare_cached(
                "SELECT s.id, s.repo, s.name, s.kind, s.signature,
                        s.start_line, s.start_col, s.end_line, s.end_col,
                        s.is_exported, s.complexity, f.path
                 FROM symbols s
                 JOIN files f ON s.file_id = f.id
                 WHERE s.name = ?1
                 ORDER BY s.is_exported DESC, s.id ASC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![sym, limit as i64], map_node_row)?;
            let mut nodes = Vec::new();
            for r in rows {
                nodes.push(r?);
            }
            Ok(nodes)
        }
        ViewSeed::Impact(sym) => {
            let mut stmt = conn.prepare_cached(
                "SELECT s.id, s.repo, s.name, s.kind, s.signature,
                        s.start_line, s.start_col, s.end_line, s.end_col,
                        s.is_exported, s.complexity, f.path
                 FROM symbols s
                 JOIN files f ON s.file_id = f.id
                 WHERE s.name = ?1
                 ORDER BY s.is_exported DESC, s.id ASC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![sym, limit as i64], map_node_row)?;
            let mut nodes = Vec::new();
            for r in rows {
                nodes.push(r?);
            }
            Ok(nodes)
        }
    }
}

fn fetch_neighbor_nodes(conn: &Connection, node_id: i64) -> Result<Vec<ViewerNode>, ViewError> {
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT s.id, s.repo, s.name, s.kind, s.signature,
                s.start_line, s.start_col, s.end_line, s.end_col,
                s.is_exported, s.complexity, f.path
         FROM (
             SELECT to_symbol_id AS neighbor_id FROM edges WHERE from_symbol_id = ?1 AND to_symbol_id IS NOT NULL
             UNION
             SELECT from_symbol_id AS neighbor_id FROM edges WHERE to_symbol_id = ?1 AND from_symbol_id IS NOT NULL
         ) n
         JOIN symbols s ON n.neighbor_id = s.id
         JOIN files f ON s.file_id = f.id
         ORDER BY s.is_exported DESC, s.id ASC",
    )?;
    let rows = stmt.query_map(params![node_id], map_node_row)?;
    let mut neighbors = Vec::new();
    for r in rows {
        neighbors.push(r?);
    }
    Ok(neighbors)
}

fn fetch_connecting_edges(
    conn: &Connection,
    node_ids: &HashSet<i64>,
) -> Result<Vec<ViewerEdge>, ViewError> {
    if node_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut sorted_ids: Vec<i64> = node_ids.iter().copied().collect();
    sorted_ids.sort_unstable();

    let mut edges = Vec::new();
    // Chunking to avoid SQLite host parameter limits if set size is large
    for chunk in sorted_ids.chunks(400) {
        let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT from_symbol_id, to_symbol_id, kind, provenance, confidence
             FROM edges
             WHERE from_symbol_id IN ({placeholders})
               AND to_symbol_id IS NOT NULL
             ORDER BY id ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
            let from: i64 = row.get(0)?;
            let to: i64 = row.get(1)?;
            let kind_str: String = row.get(2)?;
            let prov_str: String = row.get(3)?;
            let confidence: f64 = row.get(4)?;
            Ok((from, to, kind_str, prov_str, confidence))
        })?;

        for r in rows {
            let (from, to, kind_str, prov_str, confidence) = r?;
            if node_ids.contains(&to) {
                edges.push(ViewerEdge {
                    from,
                    to,
                    kind: parse_edge_kind(&kind_str),
                    provenance: parse_provenance(&prov_str),
                    confidence,
                });
            }
        }
    }

    Ok(edges)
}
