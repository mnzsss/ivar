//! Dead code analysis action identifying unreferenced private symbols and functions.

use thiserror::Error;

use crate::action::graph::affected::is_test_file;
use crate::domain::graph::DeadCodeItem;
use crate::store::graph::db::{GraphDb, GraphDbError};

/// Error returned during dead code analysis.
#[derive(Debug, Error)]
pub enum DeadCodeError {
    #[error("database error: {0}")]
    Db(#[from] GraphDbError),
}

/// Identifies unreferenced functions/methods within a repository or across all repositories.
///
/// Filters out test files, entry points, and exported symbols.
pub fn execute_dead_code(
    db: &GraphDb,
    repo: Option<&str>,
    limit: usize,
) -> Result<Vec<DeadCodeItem>, DeadCodeError> {
    let candidates = db.find_dead_code(repo, limit)?;

    let mut results = Vec::new();
    for (sym, file_path) in candidates {
        // Double guard: filter out test files and exported symbols if any slipped past SQL
        if is_test_file(&file_path) || sym.is_exported {
            continue;
        }

        let line = sym.span.start_line;
        results.push(DeadCodeItem {
            symbol: sym,
            file_path,
            line,
        });

        if results.len() >= limit {
            break;
        }
    }

    Ok(results)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/dead_code.rs"]
mod tests;
