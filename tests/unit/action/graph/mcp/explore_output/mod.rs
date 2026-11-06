#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use super::*;
use crate::domain::graph::{ExploreResult, Span, Symbol, SymbolKind, SymbolSnippet};
use serde_json::{Value, json};

pub(super) fn empty_result() -> ExploreResult {
    ExploreResult {
        confidence: crate::domain::graph::ExploreConfidence::High,
        query: String::new(),
        file_matches: vec![],
        primary_symbols: vec![],
        call_flows: vec![],
        impact_summary: None,
        direct_relations: vec![],
        entry_points: vec![],
        transitive_consumers: vec![],
        sources: vec![],
        flows: vec![],
        not_shown: vec![],
    }
}
pub(super) fn primary(name: &str, path: &str, code: String) -> SymbolSnippet {
    SymbolSnippet {
        weak: false,
        symbol: Symbol {
            id: Some(1),
            file_id: Some(1),
            repo: "api".into(),
            name: name.into(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(1, 1, 1, 1),
            is_exported: true,
            complexity: None,
        },
        file_path: path.into(),
        code,
        start_line: 1,
        end_line: 1,
    }
}

/// `count` numbered lines starting at `first`, each holding `filler` repeated.
pub(super) fn numbered_code(first: usize, count: usize, filler: &str) -> String {
    (first..first + count)
        .map(|n| format!("{n}\t{}", filler.repeat(12)))
        .collect::<Vec<_>>()
        .join("\n")
}

mod boundary;
mod continuation;
mod fanout;
