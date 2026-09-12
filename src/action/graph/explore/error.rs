use std::path::PathBuf;

use crate::action::graph::path::PathError;
use crate::action::graph::query::QueryError;

/// Errors that can occur during explore synthesis.
#[derive(Debug, thiserror::Error)]
pub enum ExploreError {
    #[error("Database query failed: {0}")]
    Query(#[from] QueryError),
    #[error("Path search failed: {0}")]
    Path(#[from] PathError),
    #[error("Database error: {0}")]
    Db(#[from] crate::store::graph::db::GraphDbError),
    #[error("I/O error reading source file {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}
