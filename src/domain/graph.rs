//! Pure graph domain models and code intelligence types.
//!
//! Zero I/O, zero runtime dependencies, pure serializable models.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The kind of a code symbol.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Fn,
    Method,
    Struct,
    Class,
    Trait,
    Interface,
    Enum,
    Mod,
    Const,
    Other(String),
}

/// A source code span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Span {
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

impl Span {
    pub const fn new(start_line: usize, start_col: usize, end_line: usize, end_col: usize) -> Self {
        Self {
            start_line,
            start_col,
            end_line,
            end_col,
        }
    }
}

/// A code symbol definition extracted from a source file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Symbol {
    pub id: Option<i64>,
    pub file_id: Option<i64>,
    pub repo: String,
    pub name: String,
    pub kind: SymbolKind,
    pub scope: Option<String>,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub span: Span,
    pub is_exported: bool,
}

/// The kind of relationship between symbols or code units.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Calls,
    Imports,
    Implements,
    CrossImports,
    CrossExecutes,
    CrossCallsHttp,
    Other(String),
}

/// How a relationship/edge was discovered or inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    Extracted,
    Inferred,
    Ambiguous,
}

/// A directed relationship edge between symbols or targets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Edge {
    pub id: Option<i64>,
    pub repo: String,
    pub file_id: Option<i64>,
    pub from_symbol_id: Option<i64>,
    pub to_symbol_id: Option<i64>,
    pub to_name: Option<String>,
    pub kind: EdgeKind,
    pub provenance: Provenance,
    pub line: usize,
    pub col: usize,
    pub confidence: f64,
}

/// High-level statistics for the indexed codebase graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct GraphStats {
    pub repo_count: usize,
    pub file_count: usize,
    pub symbol_count: usize,
    pub edge_count: usize,
    pub db_size_bytes: u64,
}

/// Snippet of code around a primary symbol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SymbolSnippet {
    pub symbol: Symbol,
    pub file_path: String,
    pub code: String,
    pub start_line: usize,
    pub end_line: usize,
}

/// A step in a call flow sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CallFlowItem {
    pub caller: String,
    pub callee: String,
    pub edge_kind: EdgeKind,
    pub provenance: Provenance,
    pub line: usize,
}

/// Result of an exploration query across the graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExploreResult {
    pub query: String,
    pub primary_symbols: Vec<SymbolSnippet>,
    pub call_flows: Vec<CallFlowItem>,
    pub impact_summary: Option<String>,
}

/// Result identifying impact and test files affected by changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AffectedResult {
    pub changed_files: Vec<String>,
    pub affected_test_files: Vec<String>,
}

/// A step along a path between two symbols.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PathStep {
    pub source: String,
    pub target: String,
    pub edge_kind: EdgeKind,
    pub line: usize,
}

/// Path traversal query result connecting two symbols.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PathResult {
    pub from: String,
    pub to: String,
    pub steps: Vec<PathStep>,
}

#[cfg(test)]
#[path = "../../tests/unit/domain/graph.rs"]
mod tests;
