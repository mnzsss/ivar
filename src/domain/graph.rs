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
    pub const fn new(
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
    ) -> Self {
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
mod tests {
    use super::*;

    #[test]
    fn test_symbol_kinds_and_construction() {
        let kinds = vec![
            SymbolKind::Fn,
            SymbolKind::Method,
            SymbolKind::Struct,
            SymbolKind::Class,
            SymbolKind::Trait,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::Mod,
            SymbolKind::Const,
            SymbolKind::Other("type_alias".to_string()),
        ];

        for kind in kinds {
            let sym = Symbol {
                id: Some(1),
                file_id: Some(42),
                repo: "ivar".to_string(),
                name: "test_sym".to_string(),
                kind: kind.clone(),
                scope: Some("crate::domain".to_string()),
                signature: Some("fn test_sym() -> ()".to_string()),
                docstring: Some("A doc comment".to_string()),
                span: Span::new(10, 1, 20, 1),
                is_exported: true,
            };
            assert_eq!(sym.kind, kind);
            assert!(sym.is_exported);
            assert_eq!(sym.span.start_line, 10);
            assert_eq!(sym.span.end_line, 20);
        }
    }

    #[test]
    fn test_edge_kinds_provenance_and_construction() {
        let kinds = vec![
            EdgeKind::Calls,
            EdgeKind::Imports,
            EdgeKind::Implements,
            EdgeKind::CrossImports,
            EdgeKind::CrossExecutes,
            EdgeKind::CrossCallsHttp,
            EdgeKind::Other("dynamic_dispatch".to_string()),
        ];

        let provenances = vec![
            Provenance::Extracted,
            Provenance::Inferred,
            Provenance::Ambiguous,
        ];

        for kind in kinds {
            for &prov in &provenances {
                let edge = Edge {
                    id: Some(10),
                    repo: "ivar".to_string(),
                    file_id: Some(42),
                    from_symbol_id: Some(1),
                    to_symbol_id: Some(2),
                    to_name: Some("target_fn".to_string()),
                    kind: kind.clone(),
                    provenance: prov,
                    line: 15,
                    col: 5,
                    confidence: 0.95,
                };
                assert_eq!(edge.kind, kind);
                assert_eq!(edge.provenance, prov);
                assert!((edge.confidence - 0.95).abs() < f64::EPSILON);
            }
        }
    }

    #[test]
    fn test_graph_stats_json_roundtrip() {
        let stats = GraphStats {
            repo_count: 3,
            file_count: 120,
            symbol_count: 1500,
            edge_count: 4200,
            db_size_bytes: 1048576,
        };

        let json = serde_json::to_string(&stats).expect("serialize stats");
        let deserialized: GraphStats = serde_json::from_str(&json).expect("deserialize stats");
        assert_eq!(stats, deserialized);
    }

    #[test]
    fn test_explore_result_json_roundtrip() {
        let explore = ExploreResult {
            query: "Symbol".to_string(),
            primary_symbols: vec![SymbolSnippet {
                symbol: Symbol {
                    id: Some(1),
                    file_id: Some(1),
                    repo: "ivar".to_string(),
                    name: "Symbol".to_string(),
                    kind: SymbolKind::Struct,
                    scope: None,
                    signature: Some("pub struct Symbol".to_string()),
                    docstring: None,
                    span: Span::new(1, 1, 10, 1),
                    is_exported: true,
                },
                file_path: "src/domain/graph.rs".to_string(),
                code: "pub struct Symbol { ... }".to_string(),
                start_line: 1,
                end_line: 10,
            }],
            call_flows: vec![CallFlowItem {
                caller: "main".to_string(),
                callee: "init".to_string(),
                edge_kind: EdgeKind::Calls,
                provenance: Provenance::Extracted,
                line: 42,
            }],
            impact_summary: Some("Core domain model".to_string()),
        };

        let json = serde_json::to_string(&explore).expect("serialize explore");
        let deserialized: ExploreResult = serde_json::from_str(&json).expect("deserialize explore");
        assert_eq!(explore, deserialized);
    }

    #[test]
    fn test_affected_result_json_roundtrip() {
        let affected = AffectedResult {
            changed_files: vec!["src/domain/graph.rs".to_string()],
            affected_test_files: vec!["tests/graph_test.rs".to_string()],
        };

        let json = serde_json::to_string(&affected).expect("serialize affected");
        let deserialized: AffectedResult = serde_json::from_str(&json).expect("deserialize affected");
        assert_eq!(affected, deserialized);
    }

    #[test]
    fn test_path_result_json_roundtrip() {
        let path = PathResult {
            from: "main".to_string(),
            to: "execute".to_string(),
            steps: vec![PathStep {
                source: "main".to_string(),
                target: "execute".to_string(),
                edge_kind: EdgeKind::Calls,
                line: 55,
            }],
        };

        let json = serde_json::to_string(&path).expect("serialize path");
        let deserialized: PathResult = serde_json::from_str(&json).expect("deserialize path");
        assert_eq!(path, deserialized);
    }
}
