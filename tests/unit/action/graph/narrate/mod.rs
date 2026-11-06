//! Unit tests for decision-oriented exploration rendering.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::graph::query::{
    CalleeInfo, CallerInfo, FileOutline, ImpactItem, ImpactResult, ReferenceSite, SymbolLocation,
};
use crate::domain::graph::{
    Edge, EdgeKind, ExploreImpact, FileMatch, FileMatchKind, FileMention, MentionedSymbol,
    OperationalRelation, PathResult, PathStep, Provenance, RelationDirection, RelationEndpoint,
    SourceExcerpt, SourceFile, Span, Symbol, SymbolKind, SymbolSnippet,
};

pub(super) fn symbol(name: &str, repo: &str, line: usize) -> Symbol {
    Symbol {
        id: Some(1),
        file_id: Some(1),
        repo: repo.into(),
        name: name.into(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: None,
        docstring: None,
        span: Span {
            start_line: line,
            start_col: 0,
            end_line: line + 2,
            end_col: 1,
        },
        is_exported: true,
        complexity: None,
    }
}

pub(super) fn snippet(name: &str, repo: &str, file: &str, line: usize) -> SymbolSnippet {
    SymbolSnippet {
        weak: false,
        symbol: symbol(name, repo, line),
        file_path: file.into(),
        code: format!("{line}\tfn {name}() {{}}"),
        start_line: line,
        end_line: line + 2,
    }
}

pub(super) fn source(repo: &str, file: &str, lines: usize) -> SourceFile {
    let code = (1..=lines)
        .map(|n| format!("{n}\tline {n} of {file}"))
        .collect::<Vec<_>>()
        .join("\n");
    SourceFile {
        repo: repo.into(),
        file_path: file.into(),
        line_count: lines,
        excerpts: vec![SourceExcerpt {
            start_line: 1,
            end_line: lines,
            code,
        }],
        changed_since_index: false,
    }
}

pub(super) fn relation(
    caller: &str,
    caller_file: &str,
    caller_repo: &str,
    target: &str,
    target_file: &str,
    line: usize,
    provenance: Provenance,
) -> OperationalRelation {
    OperationalRelation {
        source: RelationEndpoint {
            repo: caller_repo.into(),
            file_path: caller_file.into(),
            symbol_name: caller.into(),
            symbol_kind: Some(SymbolKind::Fn),
        },
        target: RelationEndpoint {
            repo: "api".into(),
            file_path: target_file.into(),
            symbol_name: target.into(),
            symbol_kind: Some(SymbolKind::Fn),
        },
        direction: RelationDirection::Incoming,
        edge_kind: EdgeKind::Calls,
        provenance,
        confidence: if matches!(provenance, Provenance::Extracted) {
            1.0
        } else {
            0.55
        },
        line,
        hop_count: 1,
        cross_repo: caller_repo != "api",
    }
}

pub(super) fn result(query: &str) -> ExploreResult {
    ExploreResult {
        confidence: crate::domain::graph::ExploreConfidence::High,
        file_matches: Vec::new(),
        query: query.into(),
        primary_symbols: Vec::new(),
        call_flows: Vec::new(),
        impact_summary: None,
        direct_relations: Vec::new(),
        entry_points: Vec::new(),
        transitive_consumers: Vec::new(),
        sources: Vec::new(),
        flows: Vec::new(),
        not_shown: Vec::new(),
    }
}

pub(super) fn caller(name: &str, file: &str, line: usize, provenance: Provenance) -> CallerInfo {
    CallerInfo {
        caller: symbol(name, "web", 1),
        caller_file_path: file.into(),
        edge_kind: EdgeKind::Calls,
        provenance,
        confidence: if matches!(provenance, Provenance::Extracted) {
            0.95
        } else {
            0.7
        },
        line,
        col: 5,
    }
}

mod budget;
mod explore;
mod lists;
