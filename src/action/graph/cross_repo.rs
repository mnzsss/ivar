//! Cross-repository hall relationship linking.
//!
//! Synchronously detects cross-repo dependencies, imports, CLI executions, and HTTP calls
//! across all mounted repositories in the hall.

use std::collections::HashMap;

use rusqlite::params;
use thiserror::Error;

use crate::store::graph::db::GraphDb;

/// Error type for cross-repo linking operations.
#[derive(Debug, Error)]
pub enum CrossRepoError {
    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("Graph DB error: {0}")]
    GraphDb(#[from] crate::store::graph::db::GraphDbError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result summary of linking cross-repository edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CrossRepoLinkOutcome {
    pub cross_imports: usize,
    pub cross_executes: usize,
    pub cross_calls_http: usize,
    pub total_linked: usize,
}

/// Links unresolved dangling edges across repositories in the codebase graph.
///
/// Discovers:
/// 1. Cross-repo imports (`CROSS_IMPORTS`): Unresolved imports/calls targeting exported symbols in other repos.
/// 2. Cross-repo CLI executions (`CROSS_EXECUTES`): Command strings/invocations matching CLI binary or subcommand names in other repos.
/// 3. Cross-repo HTTP calls (`CROSS_CALLS_HTTP`): HTTP URL path strings matching route endpoints/handlers in other repos.
pub fn link_cross_repo_edges(db: &GraphDb) -> Result<CrossRepoLinkOutcome, CrossRepoError> {
    let conn = db.conn();

    conn.execute_batch("BEGIN IMMEDIATE;")?;
    let res = (|| -> Result<CrossRepoLinkOutcome, CrossRepoError> {
        let mut out = CrossRepoLinkOutcome::default();

        // 1. Cross-Import Linking:
        // Match dangling edges (to_symbol_id IS NULL AND to_name IS NOT NULL)
        // against exported symbols in a *different* repo (symbols.repo != edges.repo AND symbols.is_exported = 1).
        // If to_name matches symbols.name, link to_symbol_id, set kind = 'CROSS_IMPORTS',
        // provenance = 'INFERRED', confidence = 0.90.
        let cross_imports = conn.execute(
            "UPDATE edges
             SET to_symbol_id = (
                 SELECT s.id FROM symbols s
                 WHERE s.repo != edges.repo
                   AND s.is_exported = 1
                   AND s.name = edges.to_name
                 LIMIT 1
             ),
             kind = 'CROSS_IMPORTS',
             provenance = 'INFERRED',
             confidence = 0.90
             WHERE to_symbol_id IS NULL
               AND to_name IS NOT NULL
               AND (kind = 'IMPORTS' OR kind = 'imports' OR kind = 'CROSS_IMPORTS' OR kind = 'cross_imports')
               AND EXISTS (
                   SELECT 1 FROM symbols s
                   WHERE s.repo != edges.repo
                     AND s.is_exported = 1
                     AND s.name = edges.to_name
               )",
            [],
        )?;
        out.cross_imports = cross_imports;

        // 2. Cross-Execute Linking:
        // Look for edges where to_name is a known binary name or CLI command pattern
        // (or kind is already CROSS_EXECUTES or to_name matches binary/CLI symbols in other repos).
        // Matches target symbols with kind in ('fn', 'const', 'mod', 'struct') and name matching to_name,
        // or CLI entry points where symbols.name = edges.to_name in other repos.
        let cross_executes = conn.execute(
            "UPDATE edges
             SET to_symbol_id = (
                 SELECT s.id FROM symbols s
                 WHERE s.repo != edges.repo
                   AND s.name = edges.to_name
                 LIMIT 1
             ),
             kind = 'CROSS_EXECUTES',
             provenance = 'INFERRED',
             confidence = 0.85
             WHERE to_symbol_id IS NULL
               AND to_name IS NOT NULL
               AND (kind = 'CROSS_EXECUTES' OR kind = 'cross_executes' OR to_name IN ('ivar', 'orca', 'valhalla', 'cargo', 'npm', 'sh', 'exec'))
               AND EXISTS (
                   SELECT 1 FROM symbols s
                   WHERE s.repo != edges.repo
                     AND s.name = edges.to_name
               )",
            [],
        )?;
        out.cross_executes = cross_executes;

        // 3. Cross-HTTP Linking:
        // Look for edges where to_name or route string matches HTTP routes or endpoint handler symbols.
        let cross_calls_http = conn.execute(
            "UPDATE edges
             SET to_symbol_id = (
                 SELECT s.id FROM symbols s
                 WHERE s.repo != edges.repo
                   AND (s.name = edges.to_name OR s.scope = edges.to_name)
                 LIMIT 1
             ),
             kind = 'CROSS_CALLS_HTTP',
             provenance = 'INFERRED',
             confidence = 0.80
             WHERE to_symbol_id IS NULL
               AND to_name IS NOT NULL
               AND (kind = 'CROSS_CALLS_HTTP' OR kind = 'cross_calls_http' OR to_name LIKE '/api/%' OR to_name LIKE 'http%')
               AND EXISTS (
                   SELECT 1 FROM symbols s
                   WHERE s.repo != edges.repo
                     AND (s.name = edges.to_name OR s.scope = edges.to_name)
               )",
            [],
        )?;
        out.cross_calls_http = cross_calls_http;

        // 4. Client calls with path parameters, in any repo: the client's
        // `GET /projects/:param` reaches the server's `GET /projects/:id`.
        let mut route_by_key: HashMap<String, i64> = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT id, name FROM symbols WHERE kind = 'route'")?;
            let routes = stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            for route in routes {
                let (id, name) = route?;
                route_by_key.entry(route_key(&name)).or_insert(id);
            }
        }
        let calls: Vec<(i64, String)> = conn
            .prepare(
                "SELECT id, to_name FROM edges
                 WHERE to_symbol_id IS NULL
                   AND to_name IS NOT NULL
                   AND kind IN ('CROSS_CALLS_HTTP', 'cross_calls_http')",
            )?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut link = conn.prepare(
            "UPDATE edges SET to_symbol_id = ?1, provenance = 'INFERRED', confidence = 0.80
             WHERE id = ?2",
        )?;
        for (edge_id, to_name) in calls {
            if let Some(route_id) = route_by_key.get(&route_key(&to_name)) {
                link.execute(params![route_id, edge_id])?;
                out.cross_calls_http += 1;
            }
        }

        out.total_linked = out.cross_imports + out.cross_executes + out.cross_calls_http;
        Ok(out)
    })();

    match res {
        Ok(outcome) => {
            conn.execute_batch("COMMIT;")?;
            Ok(outcome)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(e)
        }
    }
}

/// Writes every path parameter as `*`, so `GET /projects/:id`,
/// `GET /projects/{id}` and a client's `GET /projects/:param` share one key.
fn route_key(name: &str) -> String {
    let (method, path) = name.split_once(' ').unwrap_or(("", name));
    let segments: Vec<&str> = path
        .trim_end_matches('/')
        .split('/')
        .map(|segment| {
            if segment.starts_with(':') || segment.starts_with('{') || segment == "*" {
                "*"
            } else {
                segment
            }
        })
        .collect();
    format!("{} {}", method.to_ascii_uppercase(), segments.join("/"))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/cross_repo.rs"]
mod tests;
