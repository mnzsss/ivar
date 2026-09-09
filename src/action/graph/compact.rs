//! Positional pipe-delimited compact encoding with `#SCHEMA:` headers.

use crate::action::graph::query::{CalleeInfo, CallerInfo, ImpactResult, SymbolLocation};
use crate::domain::graph::{
    AffectedResult, ComplexityItem, DeadCodeItem, ExploreResult, HierarchyItem, PathResult,
};
use crate::store::graph::db::{edge_kind_to_str, symbol_kind_to_str};

pub const SYMBOL_SCHEMA: &str = "#SCHEMA: id|name|kind|file|line|col|complexity";
pub const DEAD_CODE_SCHEMA: &str = "#SCHEMA: name|kind|file|line";
pub const COMPLEXITY_SCHEMA: &str = "#SCHEMA: complexity|name|kind|file|line";
pub const HIERARCHY_SCHEMA: &str = "#SCHEMA: symbol|kind|file|bases|implementations";
pub const CALLER_SCHEMA: &str = "#SCHEMA: caller|kind|file|line|edge_kind|confidence";
pub const CALLEE_SCHEMA: &str = "#SCHEMA: callee|kind|file|line|edge_kind|confidence";
pub const IMPACT_SCHEMA: &str = "#SCHEMA: symbol|kind|file|depth|path_via";
pub const AFFECTED_SCHEMA: &str = "#SCHEMA: test_file";
pub const AFFECTED_RECOMMENDATION_SCHEMA: &str = "#SCHEMA: repo|test_file|direct_change|hop_count|edge_kind|provenance|confidence|command|reason|causal_path";
pub const PATH_SCHEMA: &str = "#SCHEMA: step|symbol|kind|file|edge_kind";
pub const RELATION_SCHEMA: &str = "#SCHEMA: source_symbol|source_repo|source_file|direction|target_symbol|target_repo|target_file|edge_kind|provenance|confidence|line|hop_count|cross_repo";
pub const EXPLORE_IMPACT_SCHEMA: &str = "#SCHEMA: symbol|repo|file|depth|path_via|cross_repo";
/// Trait for converting domain types and results to compact pipe-delimited string representations.
pub trait ToCompact {
    fn to_compact(&self) -> String;
}

/// Encodes a list of symbol locations:
/// `#SCHEMA: id|name|kind|file|line|col|complexity`
pub fn encode_symbols(symbols: &[SymbolLocation]) -> String {
    let mut out = String::from(SYMBOL_SCHEMA);
    for item in symbols {
        let sym = &item.symbol;
        let id_str = sym.id.map_or_else(String::new, |id| id.to_string());
        let kind = symbol_kind_to_str(&sym.kind);
        let complexity_str = sym.complexity.map_or_else(String::new, |c| c.to_string());
        out.push('\n');
        out.push_str(&format!(
            "{}|{}|{}|{}|{}|{}|{}",
            id_str,
            sym.name,
            kind,
            item.file_path,
            sym.span.start_line,
            sym.span.start_col,
            complexity_str
        ));
    }
    out
}

/// Encodes dead code candidate items:
/// `#SCHEMA: name|kind|file|line`
pub fn encode_dead_code(items: &[DeadCodeItem]) -> String {
    let mut out = String::from(DEAD_CODE_SCHEMA);
    for item in items {
        let kind = symbol_kind_to_str(&item.symbol.kind);
        out.push('\n');
        out.push_str(&format!(
            "{}|{}|{}|{}",
            item.symbol.name, kind, item.file_path, item.line
        ));
    }
    out
}

/// Encodes cyclomatic complexity items:
/// `#SCHEMA: complexity|name|kind|file|line`
pub fn encode_complexity(items: &[ComplexityItem]) -> String {
    let mut out = String::from(COMPLEXITY_SCHEMA);
    for item in items {
        let kind = symbol_kind_to_str(&item.symbol.kind);
        out.push('\n');
        out.push_str(&format!(
            "{}|{}|{}|{}|{}",
            item.complexity, item.symbol.name, kind, item.file_path, item.line
        ));
    }
    out
}

/// Encodes class/struct hierarchy item:
/// `#SCHEMA: symbol|kind|file|bases|implementations`
pub fn encode_hierarchy(item: Option<&HierarchyItem>) -> String {
    let mut out = String::from(HIERARCHY_SCHEMA);
    if let Some(h) = item {
        let kind = symbol_kind_to_str(&h.symbol.kind);
        let bases = h.bases.join(",");
        let impls = h.implementations.join(",");
        out.push('\n');
        out.push_str(&format!(
            "{}|{}|{}|{}|{}",
            h.symbol.name, kind, h.file_path, bases, impls
        ));
    }
    out
}

/// Encodes caller references:
/// `#SCHEMA: caller|kind|file|line|edge_kind|confidence`
pub fn encode_callers(callers: &[CallerInfo]) -> String {
    let mut out = String::from(CALLER_SCHEMA);
    for c in callers {
        let kind = symbol_kind_to_str(&c.caller.kind);
        let edge_kind = edge_kind_to_str(&c.edge_kind);
        out.push('\n');
        out.push_str(&format!(
            "{}|{}|{}|{}|{}|{:.2}",
            c.caller.name, kind, c.caller_file_path, c.line, edge_kind, c.confidence
        ));
    }
    out
}

/// Encodes callee references:
/// `#SCHEMA: callee|kind|file|line|edge_kind|confidence`
pub fn encode_callees(callees: &[CalleeInfo]) -> String {
    let mut out = String::from(CALLEE_SCHEMA);
    for c in callees {
        let kind = c
            .callee_symbol
            .as_ref()
            .map_or_else(|| "".into(), |s| symbol_kind_to_str(&s.kind));
        let file = c.callee_file_path.as_deref().unwrap_or("");
        let edge_kind = edge_kind_to_str(&c.edge_kind);
        out.push('\n');
        out.push_str(&format!(
            "{}|{}|{}|{}|{}|{:.2}",
            c.callee_name, kind, file, c.line, edge_kind, c.confidence
        ));
    }
    out
}

/// Encodes blast-radius impact query result:
/// `#SCHEMA: symbol|kind|file|depth|path_via`
pub fn encode_impact(impact: &ImpactResult) -> String {
    let mut out = String::from(IMPACT_SCHEMA);
    for item in &impact.affected_symbols {
        let kind = symbol_kind_to_str(&item.symbol.kind);
        let path_via = item.path_via.join(",");
        out.push('\n');
        out.push_str(&format!(
            "{}|{}|{}|{}|{}",
            item.symbol.name, kind, item.file_path, item.depth, path_via
        ));
    }
    out
}

/// Encodes affected test files:
/// `#SCHEMA: test_file`
pub fn encode_affected(affected: &AffectedResult) -> String {
    let mut out = String::from(AFFECTED_SCHEMA);
    for test_file in &affected.affected_test_files {
        out.push('\n');
        out.push_str(test_file);
    }
    if !affected.recommendations.is_empty() {
        out.push('\n');
        out.push_str(AFFECTED_RECOMMENDATION_SCHEMA);
        for rec in &affected.recommendations {
            let cmd_str = rec.command.as_deref().unwrap_or("");
            let causal_str: Vec<String> = rec
                .causal_path
                .iter()
                .map(|step| {
                    format!(
                        "{} -[{}]-> {}",
                        step.source,
                        step.edge_kind.as_str(),
                        step.target
                    )
                })
                .collect();
            let causal_path_formatted = causal_str.join(" ; ");
            out.push('\n');
            out.push_str(&format!(
                "{}|{}|{}|{}|{}|{}|{:.2}|{}|{}|{}",
                rec.repo,
                rec.test_file,
                rec.direct_change,
                rec.hop_count,
                rec.edge_kind.as_str(),
                rec.provenance.as_str(),
                rec.confidence,
                cmd_str,
                rec.reason,
                causal_path_formatted
            ));
        }
    }
    out
}

/// Encodes path between symbols:
/// `#SCHEMA: step|symbol|kind|file|edge_kind`
pub fn encode_path(path: Option<&PathResult>) -> String {
    let mut out = String::from(PATH_SCHEMA);
    if let Some(p) = path {
        for (idx, step) in p.steps.iter().enumerate() {
            let edge_kind = edge_kind_to_str(&step.edge_kind);
            out.push('\n');
            out.push_str(&format!("{}|{}|||{}", idx + 1, step.target, edge_kind));
        }
    }
    out
}

/// Encodes explore results combining primary symbols, entry points, direct relations, and transitive consumers.
pub fn encode_explore(explore: &ExploreResult) -> String {
    let mut out = String::from(SYMBOL_SCHEMA);
    for sym_snippet in &explore.primary_symbols {
        let sym = &sym_snippet.symbol;
        let id_str = sym.id.map_or_else(String::new, |id| id.to_string());
        let kind = symbol_kind_to_str(&sym.kind);
        let complexity_str = sym.complexity.map_or_else(String::new, |c| c.to_string());
        out.push('\n');
        out.push_str(&format!(
            "{}|{}|{}|{}|{}|{}|{}",
            id_str,
            sym.name,
            kind,
            sym_snippet.file_path,
            sym.span.start_line,
            sym.span.start_col,
            complexity_str
        ));
    }
    if !explore.direct_relations.is_empty() {
        out.push('\n');
        out.push_str(RELATION_SCHEMA);
        for rel in &explore.direct_relations {
            let dir_str = match rel.direction {
                crate::domain::graph::RelationDirection::Incoming => "incoming",
                crate::domain::graph::RelationDirection::Outgoing => "outgoing",
            };
            let edge_str = rel.edge_kind.as_str();
            let prov_str = rel.provenance.as_str();
            out.push('\n');
            out.push_str(&format!(
                "{}|{}|{}|{}|{}|{}|{}|{}|{}|{:.2}|{}|{}|{}",
                rel.source.symbol_name,
                rel.source.repo,
                rel.source.file_path,
                dir_str,
                rel.target.symbol_name,
                rel.target.repo,
                rel.target.file_path,
                edge_str,
                prov_str,
                rel.confidence,
                rel.line,
                rel.hop_count,
                rel.cross_repo
            ));
        }
    }
    if !explore.transitive_consumers.is_empty() {
        out.push('\n');
        out.push_str(EXPLORE_IMPACT_SCHEMA);
        for c in &explore.transitive_consumers {
            out.push('\n');
            out.push_str(&format!(
                "{}|{}|{}|{}|{}|{}",
                c.symbol_name,
                c.repo,
                c.file_path,
                c.depth,
                c.path_via.join(" -> "),
                c.cross_repo
            ));
        }
    }
    out
}

impl ToCompact for &[SymbolLocation] {
    fn to_compact(&self) -> String {
        encode_symbols(self)
    }
}

impl ToCompact for Vec<SymbolLocation> {
    fn to_compact(&self) -> String {
        encode_symbols(self)
    }
}

impl ToCompact for &[DeadCodeItem] {
    fn to_compact(&self) -> String {
        encode_dead_code(self)
    }
}

impl ToCompact for Vec<DeadCodeItem> {
    fn to_compact(&self) -> String {
        encode_dead_code(self)
    }
}

impl ToCompact for &[ComplexityItem] {
    fn to_compact(&self) -> String {
        encode_complexity(self)
    }
}

impl ToCompact for Vec<ComplexityItem> {
    fn to_compact(&self) -> String {
        encode_complexity(self)
    }
}

impl ToCompact for Option<&HierarchyItem> {
    fn to_compact(&self) -> String {
        encode_hierarchy(*self)
    }
}

impl ToCompact for Option<HierarchyItem> {
    fn to_compact(&self) -> String {
        encode_hierarchy(self.as_ref())
    }
}

impl ToCompact for HierarchyItem {
    fn to_compact(&self) -> String {
        encode_hierarchy(Some(self))
    }
}

impl ToCompact for &[CallerInfo] {
    fn to_compact(&self) -> String {
        encode_callers(self)
    }
}

impl ToCompact for Vec<CallerInfo> {
    fn to_compact(&self) -> String {
        encode_callers(self)
    }
}

impl ToCompact for &[CalleeInfo] {
    fn to_compact(&self) -> String {
        encode_callees(self)
    }
}

impl ToCompact for Vec<CalleeInfo> {
    fn to_compact(&self) -> String {
        encode_callees(self)
    }
}

impl ToCompact for ImpactResult {
    fn to_compact(&self) -> String {
        encode_impact(self)
    }
}

impl ToCompact for &ImpactResult {
    fn to_compact(&self) -> String {
        encode_impact(self)
    }
}

impl ToCompact for AffectedResult {
    fn to_compact(&self) -> String {
        encode_affected(self)
    }
}

impl ToCompact for &AffectedResult {
    fn to_compact(&self) -> String {
        encode_affected(self)
    }
}

impl ToCompact for PathResult {
    fn to_compact(&self) -> String {
        encode_path(Some(self))
    }
}

impl ToCompact for &PathResult {
    fn to_compact(&self) -> String {
        encode_path(Some(self))
    }
}

impl ToCompact for Option<PathResult> {
    fn to_compact(&self) -> String {
        encode_path(self.as_ref())
    }
}

impl ToCompact for &Option<PathResult> {
    fn to_compact(&self) -> String {
        encode_path(self.as_ref())
    }
}

impl ToCompact for ExploreResult {
    fn to_compact(&self) -> String {
        encode_explore(self)
    }
}

impl ToCompact for &ExploreResult {
    fn to_compact(&self) -> String {
        encode_explore(self)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/compact.rs"]
mod tests;
