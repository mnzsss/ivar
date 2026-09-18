//! Domain query data structures and row mappers.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::graph::{Edge, EdgeKind, Provenance, Symbol};
use crate::store::graph::db::GraphDbError;
use crate::store::graph::db::row::symbol_from_row;

/// Error returned during graph query execution.
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("symbol not found: {0}")]
    SymbolNotFound(String),
    #[error("file not found: repo '{repo}', path '{path}'")]
    FileNotFound { repo: String, path: String },
    #[error("database error: {0}")]
    Db(#[from] GraphDbError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Symbol with associated file path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SymbolLocation {
    pub symbol: Symbol,
    pub file_path: String,
}

/// Caller information referencing a target symbol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CallerInfo {
    pub caller: Symbol,
    pub caller_file_path: String,
    pub edge_kind: EdgeKind,
    pub provenance: Provenance,
    pub confidence: f64,
    pub line: usize,
    pub col: usize,
}

/// A reference to a symbol from outside any symbol body, such as an import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReferenceSite {
    pub repo: String,
    pub file_path: String,
    pub line: usize,
}

/// Information about an outgoing call/target from a symbol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CalleeInfo {
    pub callee_name: String,
    pub callee_symbol: Option<Symbol>,
    pub callee_file_path: Option<String>,
    pub edge_kind: EdgeKind,
    pub provenance: Provenance,
    pub confidence: f64,
    pub line: usize,
    pub col: usize,
}

/// Structural outline of symbols and import dependencies in a single file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FileOutline {
    pub file_path: String,
    pub repo: String,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Edge>,
}

/// An affected symbol reached during blast-radius impact traversal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImpactItem {
    pub symbol: Symbol,
    pub file_path: String,
    pub depth: usize,
    pub path_via: Vec<String>,
}

/// Result of blast radius impact query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImpactResult {
    pub root_symbol: Symbol,
    pub affected_symbols: Vec<ImpactItem>,
    pub affected_files: Vec<String>,
    pub total_affected: usize,
}

pub(super) fn map_symbol_and_path_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(Symbol, String)> {
    let symbol = symbol_from_row(row)?;
    let file_path: String = row.get(14)?;
    Ok((symbol, file_path))
}

pub(super) fn map_symbol_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Symbol> {
    symbol_from_row(row)
}
