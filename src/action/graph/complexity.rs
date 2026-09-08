//! Cyclomatic complexity analysis action identifying high-complexity functions and methods.

use thiserror::Error;

use crate::domain::graph::ComplexityItem;
use crate::store::graph::db::{GraphDb, GraphDbError};

/// Error returned during complexity analysis.
#[derive(Debug, Error)]
pub enum ComplexityError {
    #[error("database error: {0}")]
    Db(#[from] GraphDbError),
}

/// Identifies functions and methods exceeding a cyclomatic complexity threshold.
///
/// Returned items are sorted by complexity in descending order.
pub fn execute_complexity(
    db: &GraphDb,
    repo: Option<&str>,
    threshold: u32,
    limit: usize,
) -> Result<Vec<ComplexityItem>, ComplexityError> {
    let rows = db.find_complex_symbols(repo, threshold, limit)?;

    let results = rows
        .into_iter()
        .map(|(sym, file_path)| {
            let line = sym.span.start_line;
            let complexity = sym.complexity.unwrap_or(0);
            ComplexityItem {
                symbol: sym,
                file_path,
                complexity,
                line,
            }
        })
        .collect();

    Ok(results)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/complexity.rs"]
mod tests;
