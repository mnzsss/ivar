use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::*;

/// Snippet of code around a primary symbol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SymbolSnippet {
    pub symbol: Symbol,
    pub file_path: String,
    pub code: String,
    pub start_line: usize,
    pub end_line: usize,
    /// Below the relevance threshold of the query: `code` is left empty.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub weak: bool,
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

/// The match kind of a matched file in exploration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileMatchKind {
    ExactPath,
    ExactBasename,
    PinnedPath,
    Content,
}

/// A file matched by an exploration query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileMatch {
    pub repo: String,
    pub file_path: String,
    pub match_kind: FileMatchKind,
    pub start_line: usize,
    pub excerpt: String,
    pub content_truncated: bool,
    /// Below the relevance threshold of the query: `excerpt` is left empty.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub weak: bool,
}

/// How far an exploration's candidates cover what the query asked for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ExploreConfidence {
    /// Every candidate covers the query well enough. Not printed.
    #[default]
    #[serde(rename = "high")]
    High,
    /// Some candidates are weak; they follow the strong ones.
    #[serde(rename = "partial")]
    Partial,
    /// No candidate covers the query: all are weak and carry no source.
    #[serde(rename = "none")]
    NoMatch,
}

impl ExploreConfidence {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Partial => "partial",
            Self::NoMatch => "none",
        }
    }

    #[must_use]
    pub fn is_high(&self) -> bool {
        matches!(self, Self::High)
    }
}

/// Result of an exploration query across the graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExploreResult {
    pub query: String,
    /// Left out of the answer when `high`.
    #[serde(default, skip_serializing_if = "ExploreConfidence::is_high")]
    pub confidence: ExploreConfidence,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_matches: Vec<FileMatch>,
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
