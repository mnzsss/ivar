use crate::action::graph::path::{self, PathError};
use crate::action::graph::query::SymbolLocation;
use crate::domain::graph::PathResult;
use crate::store::graph::db::GraphDb;

use super::error::ExploreError;

/// Names in one query that explore connects; every pair runs two path searches.
const MAX_NAMED_FLOW_SYMBOLS: usize = 4;
/// One symbol between two named ones at most, so a flow never fans out through a hub.
const MAX_NAMED_FLOW_HOPS: usize = 2;

/// Finds the call path between symbols the query names together, the question a
/// query like `handleLogin saveSession` usually asks.
pub(crate) fn named_flows(
    db: &GraphDb,
    query: &str,
    candidates: &[SymbolLocation],
) -> Result<Vec<PathResult>, ExploreError> {
    let mut named: Vec<&str> = Vec::new();
    for token in query.split_whitespace() {
        let name = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
        if !named.contains(&name) && candidates.iter().any(|c| c.symbol.name == name) {
            named.push(name);
        }
    }
    named.truncate(MAX_NAMED_FLOW_SYMBOLS);

    let mut flows = Vec::new();
    for (index, from) in named.iter().enumerate() {
        for to in named.iter().skip(index + 1) {
            let flow = match shortest_flow(db, from, to)? {
                Some(flow) => Some(flow),
                None => shortest_flow(db, to, from)?,
            };
            flows.extend(flow);
        }
    }
    Ok(flows)
}

fn shortest_flow(db: &GraphDb, from: &str, to: &str) -> Result<Option<PathResult>, ExploreError> {
    match path::find_shortest_path(db, from, to, MAX_NAMED_FLOW_HOPS) {
        Ok(found) => Ok(found.filter(|flow| !flow.steps.is_empty())),
        Err(PathError::StartNotFound(_) | PathError::TargetNotFound(_)) => Ok(None),
        Err(err) => Err(err.into()),
    }
}
