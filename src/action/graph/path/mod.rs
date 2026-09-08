//! Bidirectional BFS shortest-path finder between symbols or files.
//!
//! Synchronously traverses graph edges in both directions (forward from source,
//! backward from destination) until frontiers intersect or `max_hops` is reached.

mod db;

use std::collections::{HashMap, VecDeque};

use thiserror::Error;
#[cfg(test)]
use rusqlite::params;

use self::db::{get_incoming_edges, get_outgoing_edges, resolve_candidates};
pub use crate::domain::graph::{PathResult, PathStep};
use crate::store::graph::db::GraphDb;

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

/// Finds the shortest directed path between two symbols or files using bidirectional BFS.
pub fn find_shortest_path(
    db: &GraphDb,
    from: &str,
    to: &str,
    max_hops: usize,
) -> Result<Option<PathResult>, PathError> {
    if max_hops == 0 {
        return Err(PathError::InvalidMaxHops(0));
    }


    let conn = db.conn();
    let start_candidates = resolve_candidates(conn, from)?;
    if start_candidates.is_empty() {
        return Err(PathError::StartNotFound(from.to_owned()));
    }

    // 2. Resolve target candidates
    let target_candidates = resolve_candidates(conn, to)?;
    if target_candidates.is_empty() {
        return Err(PathError::TargetNotFound(to.to_owned()));
    }

    // Direct overlap check
    for target in &target_candidates {
        if start_candidates.contains(target) {
            return Ok(Some(PathResult {
                from: from.to_owned(),
                to: to.to_owned(),
                steps: Vec::new(),
            }));
        }
    }

    // 3. Setup bidirectional BFS
    let mut forward_queue = VecDeque::new();
    let mut backward_queue = VecDeque::new();

    // Maps: node_id -> Option<(predecessor_node_id, step_taken)>
    let mut forward_visited: HashMap<i64, Option<(i64, PathStep)>> = HashMap::new();
    let mut backward_visited: HashMap<i64, Option<(i64, PathStep)>> = HashMap::new();

    for &s in &start_candidates {
        forward_visited.insert(s, None);
        forward_queue.push_back((s, 0));
    }

    for &t in &target_candidates {
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

    // 4. Reconstruct path from forward_visited and backward_visited
    let steps = reconstruct_bidirectional_path(meeting_node, &forward_visited, &backward_visited);

    if steps.len() > max_hops {
        return Ok(None);
    }

    Ok(Some(PathResult {
        from: from.to_owned(),
        to: to.to_owned(),
        steps,
    }))
}

fn reconstruct_bidirectional_path(
    meeting_node: i64,
    forward_visited: &HashMap<i64, Option<(i64, PathStep)>>,
    backward_visited: &HashMap<i64, Option<(i64, PathStep)>>,
) -> Vec<PathStep> {
    let mut steps = Vec::new();

    // Trace forward: from meeting_node back to start
    let mut curr = meeting_node;
    let mut forward_steps = Vec::new();
    while let Some(Some((prev, step))) = forward_visited.get(&curr) {
        forward_steps.push(step.clone());
        curr = *prev;
    }
    forward_steps.reverse();
    steps.extend(forward_steps);

    // Trace backward: from meeting_node forward to target
    let mut curr = meeting_node;
    while let Some(Some((next, step))) = backward_visited.get(&curr) {
        steps.push(step.clone());
        curr = *next;
    }

    steps
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/path.rs"]
mod tests;
