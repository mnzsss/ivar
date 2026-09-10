//! Pure graph domain models and code intelligence types.
//!
//! Zero I/O, zero runtime dependencies, pure serializable models.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The kind of a code symbol.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(into = "String", from = "String")]
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

impl SymbolKind {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Fn => "fn",
            Self::Method => "method",
            Self::Struct => "struct",
            Self::Class => "class",
            Self::Trait => "trait",
            Self::Interface => "interface",
            Self::Enum => "enum",
            Self::Mod => "mod",
            Self::Const => "const",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<SymbolKind> for String {
    fn from(k: SymbolKind) -> Self {
        k.as_str().to_owned()
    }
}

impl From<&SymbolKind> for String {
    fn from(k: &SymbolKind) -> Self {
        k.as_str().to_owned()
    }
}

impl From<String> for SymbolKind {
    fn from(s: String) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "fn" => Self::Fn,
            "method" => Self::Method,
            "struct" => Self::Struct,
            "class" => Self::Class,
            "trait" => Self::Trait,
            "interface" => Self::Interface,
            "enum" => Self::Enum,
            "mod" => Self::Mod,
            "const" => Self::Const,
            _ => Self::Other(s),
        }
    }
}

impl From<&str> for SymbolKind {
    fn from(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "fn" => Self::Fn,
            "method" => Self::Method,
            "struct" => Self::Struct,
            "class" => Self::Class,
            "trait" => Self::Trait,
            "interface" => Self::Interface,
            "enum" => Self::Enum,
            "mod" => Self::Mod,
            "const" => Self::Const,
            _ => Self::Other(s.to_owned()),
        }
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complexity: Option<u32>,
}

/// The kind of relationship between symbols or code units.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(into = "String", from = "String")]
pub enum EdgeKind {
    Calls,
    Imports,
    Implements,
    Inherits,
    References,
    CrossImports,
    CrossExecutes,
    CrossCallsHttp,
    Other(String),
}

impl EdgeKind {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Calls => "calls",
            Self::Imports => "imports",
            Self::Implements => "implements",
            Self::Inherits => "inherits",
            Self::References => "references",
            Self::CrossImports => "cross_imports",
            Self::CrossExecutes => "cross_executes",
            Self::CrossCallsHttp => "cross_calls_http",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl std::fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<EdgeKind> for String {
    fn from(k: EdgeKind) -> Self {
        k.as_str().to_owned()
    }
}

impl From<&EdgeKind> for String {
    fn from(k: &EdgeKind) -> Self {
        k.as_str().to_owned()
    }
}

impl From<String> for EdgeKind {
    fn from(s: String) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "calls" => Self::Calls,
            "imports" => Self::Imports,
            "implements" => Self::Implements,
            "inherits" => Self::Inherits,
            "references" => Self::References,
            "cross_imports" => Self::CrossImports,
            "cross_executes" => Self::CrossExecutes,
            "cross_calls_http" => Self::CrossCallsHttp,
            _ => Self::Other(s),
        }
    }
}

impl From<&str> for EdgeKind {
    fn from(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "calls" => Self::Calls,
            "imports" => Self::Imports,
            "implements" => Self::Implements,
            "inherits" => Self::Inherits,
            "references" => Self::References,
            "cross_imports" => Self::CrossImports,
            "cross_executes" => Self::CrossExecutes,
            "cross_calls_http" => Self::CrossCallsHttp,
            _ => Self::Other(s.to_owned()),
        }
    }
}

/// How a relationship/edge was discovered or inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    Extracted,
    Inferred,
    Ambiguous,
}

impl Provenance {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Extracted => "extracted",
            Self::Inferred => "inferred",
            Self::Ambiguous => "ambiguous",
        }
    }
}

impl std::fmt::Display for Provenance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
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
/// Direction of an operational relation relative to the primary symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RelationDirection {
    Incoming,
    Outgoing,
}

/// Endpoint descriptor for an operational relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RelationEndpoint {
    pub repo: String,
    pub file_path: String,
    pub symbol_name: String,
    pub symbol_kind: Option<SymbolKind>,
}

/// An operational, evidence-backed relationship between code units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OperationalRelation {
    pub source: RelationEndpoint,
    pub target: RelationEndpoint,
    pub direction: RelationDirection,
    pub edge_kind: EdgeKind,
    pub provenance: Provenance,
    pub confidence: f64,
    pub line: usize,
    pub hop_count: usize,
    pub cross_repo: bool,
}

/// Bounded impact evidence for a primary symbol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExploreImpact {
    pub symbol_name: String,
    pub repo: String,
    pub file_path: String,
    pub depth: usize,
    pub path_via: Vec<String>,
    pub cross_repo: bool,
}

/// Source explore shows for one file: the whole file, or merged excerpts around
/// the matched symbols when the file is large.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SourceFile {
    pub repo: String,
    pub file_path: String,
    pub line_count: usize,
    pub excerpts: Vec<SourceExcerpt>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub changed_since_index: bool,
}

/// A contiguous run of numbered source lines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SourceExcerpt {
    pub start_line: usize,
    pub end_line: usize,
    pub code: String,
}

/// A file that matched an exploration but got no source in the answer, named
/// with the symbols an agent can explore next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FileMention {
    pub repo: String,
    pub file_path: String,
    pub symbols: Vec<MentionedSymbol>,
}

/// A symbol named in a [`FileMention`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MentionedSymbol {
    pub name: String,
    pub line: usize,
}

/// Result of an exploration query across the graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExploreResult {
    pub query: String,
    pub primary_symbols: Vec<SymbolSnippet>,
    pub call_flows: Vec<CallFlowItem>,
    pub impact_summary: Option<String>,
    #[serde(default)]
    pub direct_relations: Vec<OperationalRelation>,
    #[serde(default)]
    pub entry_points: Vec<OperationalRelation>,
    #[serde(default)]
    pub transitive_consumers: Vec<ExploreImpact>,
    #[serde(default)]
    pub sources: Vec<SourceFile>,
    #[serde(default)]
    pub flows: Vec<PathResult>,
    #[serde(default)]
    pub not_shown: Vec<FileMention>,
}
/// A causal step linking a changed dependency or symbol to an affected test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CausalStep {
    pub source: String,
    pub target: String,
    pub edge_kind: EdgeKind,
    pub provenance: Provenance,
    pub confidence: f64,
    pub line: usize,
}

/// A verification recommendation for an affected test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AffectedRecommendation {
    pub repo: String,
    pub test_file: String,
    pub causal_path: Vec<CausalStep>,
    pub direct_change: bool,
    pub hop_count: usize,
    pub edge_kind: EdgeKind,
    pub provenance: Provenance,
    pub confidence: f64,
    pub reason: String,
    pub command: Option<String>,
}

/// Result identifying impact and test files affected by changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AffectedResult {
    pub changed_files: Vec<String>,
    pub affected_test_files: Vec<String>,
    #[serde(default)]
    pub recommendations: Vec<AffectedRecommendation>,
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

/// A dead code candidate item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeadCodeItem {
    pub symbol: Symbol,
    pub file_path: String,
    pub line: usize,
}

/// A cyclomatic complexity analysis item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ComplexityItem {
    pub symbol: Symbol,
    pub file_path: String,
    pub complexity: u32,
    pub line: usize,
}

/// A class or struct hierarchy item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HierarchyItem {
    pub symbol: Symbol,
    pub file_path: String,
    pub bases: Vec<String>,
    pub implementations: Vec<String>,
}

#[cfg(test)]
#[path = "../../tests/unit/domain/graph.rs"]
mod tests;
