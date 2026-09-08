//! Domain query data structures and row mappers.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::graph::{Edge, EdgeKind, Provenance, Span, Symbol};
use crate::store::graph::db::{GraphDbError, parse_symbol_kind};

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
    let id: i64 = row.get(0)?;
    let file_id: i64 = row.get(1)?;
    let repo: String = row.get(2)?;
    let name: String = row.get(3)?;
    let kind_raw: String = row.get(4)?;
    let scope: Option<String> = row.get(5)?;
    let signature: Option<String> = row.get(6)?;
    let docstring: Option<String> = row.get(7)?;
    let start_line: i64 = row.get(8)?;
    let start_col: i64 = row.get(9)?;
    let end_line: i64 = row.get(10)?;
    let end_col: i64 = row.get(11)?;
    let is_exported: i64 = row.get(12)?;
    let complexity: Option<u32> = row
        .get::<_, Option<i64>>(13)
        .ok()
        .flatten()
        .map(|c| c as u32);
    let file_path: String = row.get(14)?;

    Ok((
        Symbol {
            id: Some(id),
            file_id: Some(file_id),
            repo,
            name,
            kind: parse_symbol_kind(&kind_raw),
            scope,
            signature,
            docstring,
            span: Span::new(
                start_line as usize,
                start_col as usize,
                end_line as usize,
                end_col as usize,
            ),
            is_exported: is_exported != 0,
            complexity,
        },
        file_path,
    ))
}

pub(super) fn map_symbol_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Symbol> {
    let id: i64 = row.get(0)?;
    let file_id: i64 = row.get(1)?;
    let repo: String = row.get(2)?;
    let name: String = row.get(3)?;
    let kind_raw: String = row.get(4)?;
    let scope: Option<String> = row.get(5)?;
    let signature: Option<String> = row.get(6)?;
    let docstring: Option<String> = row.get(7)?;
    let start_line: i64 = row.get(8)?;
    let start_col: i64 = row.get(9)?;
    let end_line: i64 = row.get(10)?;
    let end_col: i64 = row.get(11)?;
    let is_exported: i64 = row.get(12)?;
    let complexity: Option<u32> = row
        .get::<_, Option<i64>>(13)
        .ok()
        .flatten()
        .map(|c| c as u32);

    Ok(Symbol {
        id: Some(id),
        file_id: Some(file_id),
        repo,
        name,
        kind: parse_symbol_kind(&kind_raw),
        scope,
        signature,
        docstring,
        span: Span::new(
            start_line as usize,
            start_col as usize,
            end_line as usize,
            end_col as usize,
        ),
        is_exported: is_exported != 0,
        complexity,
    })
}
