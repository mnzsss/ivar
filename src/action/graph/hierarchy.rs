//! Class, struct, interface, and trait hierarchy analysis action.

use thiserror::Error;

use crate::domain::graph::HierarchyItem;
use crate::store::graph::db::{GraphDb, GraphDbError};

/// Error returned during hierarchy analysis.
#[derive(Debug, Error)]
pub enum HierarchyError {
    #[error("database error: {0}")]
    Db(#[from] GraphDbError),
}

/// Identifies base types (implements/inherits) and derived implementations/subtypes for a symbol.
pub fn execute_hierarchy(
    db: &GraphDb,
    symbol_name: &str,
    repo: Option<&str>,
) -> Result<Option<HierarchyItem>, HierarchyError> {
    let opt = db.find_hierarchy(symbol_name, repo)?;

    Ok(opt.map(
        |(symbol, file_path, bases, implementations)| HierarchyItem {
            symbol,
            file_path,
            bases,
            implementations,
        },
    ))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/hierarchy.rs"]
mod tests;
