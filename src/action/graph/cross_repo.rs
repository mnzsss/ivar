//! Cross-repository hall relationship linking.
//!
//! Synchronously detects cross-repo dependencies, imports, CLI executions, and HTTP calls
//! across all mounted repositories in the hall.

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

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/cross_repo.rs"]
mod tests;
