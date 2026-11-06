//! Decision-oriented Markdown rendering of graph answers for MCP.
//!
//! Over MCP, a model reads these answers to decide what to open next. The
//! serialised structs spend their tokens on `file_id`, `scope: null`, and
//! `end_col`; these renderings spend them on callers, source and consequences.
//! In `benchmark2`, an agent given the raw JSON issued one `graph_explore` and
//! went back to grep.
//!
//! The compact encoding in [`super::compact`] stays a parsing target with a
//! `#SCHEMA:` header.

mod explore;
mod source;

pub use explore::{narrate_explore, narrate_requested_files};
use source::push_remainder;

use std::fmt::Write as _;

use crate::action::graph::explore::MAX_REQUESTED_FILES;
use crate::action::graph::query::{
    CalleeInfo, CallerInfo, FileOutline, ImpactResult, ReferenceSite, SymbolLocation,
};
use crate::domain::graph::{
    EdgeKind, ExploreConfidence, ExploreResult, FileMatch, FileMention, MentionedSymbol,
    Provenance, SourceFile, SymbolSnippet,
};

/// Definitions listed for a symbol before the rest are left out.
pub(super) const MAX_RELATIONS_PER_SYMBOL: usize = 6;
/// Symbols the blast radius names before counting the rest.
pub(super) const MAX_BLAST_SYMBOLS: usize = 8;
/// Caller files named on one blast-radius line.
pub(super) const MAX_CALLER_FILES: usize = 3;
/// Files the "not shown" list names before counting the rest.
pub(super) const MAX_NOT_SHOWN_FILES: usize = 10;
/// Weak candidates the answer lists before counting the rest.
pub(super) const MAX_WEAK_LISTED: usize = 12;
/// Symbol names listed for one file that got no source.
pub(super) const MAX_NAMES_PER_FILE: usize = 6;
/// Entries a callers, callees or impact answer lists before counting the rest.
pub(crate) const MAX_LIST_ITEMS: usize = 40;
/// Every later turn repeats an answer, so it stays well under the ~25K characters
/// at which hosts save a tool result to a file the agent then has to Read.
pub(crate) const MAX_OUTPUT_CHARS: usize = 18_000;
/// A `paths` answer may run longer, still under the ~25K-character threshold.
pub(super) const MAX_REQUESTED_OUTPUT_CHARS: usize = 24_000;
/// Room kept after the source for the not-shown list and the next call, so no
/// answer passes [`MAX_REQUESTED_OUTPUT_CHARS`].
pub(super) const NOT_SHOWN_RESERVE_CHARS: usize = 4_000;
/// Source keeps this much room even after long relation sections.
pub(super) const MIN_SOURCE_CHARS: usize = 8_000;
/// Renders where a symbol is defined, its call sites and type uses, and the
/// files that import it, so the answer leaves nothing for a grep to add.
pub fn narrate_callers(
    symbol: &str,
    definitions: &[SymbolLocation],
    callers: &[CallerInfo],
    references: &[ReferenceSite],
) -> String {
    let mut seen = std::collections::BTreeSet::new();
    let unique: Vec<&CallerInfo> = callers
        .iter()
        .filter(|c| {
            seen.insert((
                c.caller.repo.as_str(),
                c.caller_file_path.as_str(),
                c.caller.name.as_str(),
                c.line,
            ))
        })
        .collect();

    let mut out = String::new();
    if unique.is_empty() && references.is_empty() {
        let _ = writeln!(
            out,
            "No callers of `{symbol}` in the index. It may be unused, reached only through \
             dynamic dispatch, or newer than the last `refresh_index`."
        );
        push_definitions(&mut out, definitions);
        return out;
    }

    let several_repos = unique
        .iter()
        .map(|c| c.caller.repo.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        > 1;
    let _ = writeln!(out, "**Callers of `{symbol}`: {}**\n", unique.len());
    push_definitions(&mut out, definitions);
    for c in unique.iter().take(MAX_LIST_ITEMS) {
        let _ = write!(
            out,
            "- `{}` in {}:{}",
            c.caller.name, c.caller_file_path, c.line
        );
        if matches!(c.edge_kind, EdgeKind::References) {
            out.push_str(" · type use");
        }
        if several_repos {
            let _ = write!(out, " [repo `{}`]", c.caller.repo);
        }
        if !matches!(c.provenance, Provenance::Extracted) {
            let _ = write!(
                out,
                " — {} ({:.2}), verify this line before relying on it",
                provenance_word(c.provenance),
                c.confidence,
            );
        }
        out.push('\n');
    }
    push_remainder(&mut out, unique.len());

    if !references.is_empty() {
        let sites: Vec<String> = references
            .iter()
            .take(MAX_LIST_ITEMS)
            .map(|r| format!("{}:{}", r.file_path, r.line))
            .collect();
        let _ = writeln!(
            out,
            "\nImported or referenced outside a function at {} site{}: {}",
            references.len(),
            plural(references.len()),
            sites.join(", ")
        );
    }
    out.push_str(
        "\nThis covers every indexed file; a grep would only add files outside the index.\n",
    );
    out
}

fn push_definitions(out: &mut String, definitions: &[SymbolLocation]) {
    for d in definitions.iter().take(MAX_RELATIONS_PER_SYMBOL) {
        let _ = writeln!(
            out,
            "Defined at {}:{} ({})",
            d.file_path, d.symbol.span.start_line, d.symbol.repo
        );
    }
    if !definitions.is_empty() {
        out.push('\n');
    }
}

/// Renders the calls a symbol makes and where each one lands.
pub fn narrate_callees(symbol: &str, callees: &[CalleeInfo]) -> String {
    let mut out = String::new();
    if callees.is_empty() {
        let _ = writeln!(
            out,
            "`{symbol}` makes no calls the index recorded. If it was edited recently, call \
             `refresh_index` first."
        );
        return out;
    }

    let _ = writeln!(out, "**Calls made by `{symbol}`: {}**\n", callees.len());
    for c in callees.iter().take(MAX_LIST_ITEMS) {
        let _ = write!(out, "- line {} → `{}`", c.line, c.callee_name);
        match (&c.callee_symbol, &c.callee_file_path) {
            (Some(target), Some(path)) => {
                let _ = write!(out, " in {path}:{}", target.span.start_line);
            }
            _ => out.push_str(" (not in the index)"),
        }
        if !matches!(c.provenance, Provenance::Extracted) {
            let _ = write!(
                out,
                " — {} ({:.2})",
                provenance_word(c.provenance),
                c.confidence
            );
        }
        out.push('\n');
    }
    push_remainder(&mut out, callees.len());
    out
}

/// Renders what a change to a symbol reaches, grouped by distance.
pub fn narrate_impact(res: &ImpactResult) -> String {
    let root = &res.root_symbol;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "**Impact of `{}`: {} symbol{} in {} file{}**\n",
        root.name,
        res.total_affected,
        plural(res.total_affected),
        res.affected_files.len(),
        plural(res.affected_files.len()),
    );
    if res.affected_symbols.is_empty() {
        let _ = writeln!(out, "Nothing in the index depends on `{}`.", root.name);
        return out;
    }

    let mut items: Vec<_> = res.affected_symbols.iter().collect();
    items.sort_by_key(|item| item.depth);
    let mut depth = 0;
    for item in items.iter().take(MAX_LIST_ITEMS) {
        if item.depth != depth {
            depth = item.depth;
            let _ = writeln!(out, "Depth {depth}");
        }
        let _ = write!(
            out,
            "- `{}` {}:{}",
            item.symbol.name, item.file_path, item.symbol.span.start_line
        );
        if item.path_via.len() > 2 {
            let _ = write!(out, " via {}", item.path_via.join(" -> "));
        }
        if item.inferred {
            out.push_str(" (inferred from a name match)");
        }
        out.push('\n');
    }
    push_remainder(&mut out, items.len());
    out
}

/// Renders the symbols and imports declared in one file.
pub fn narrate_outline(outline: &FileOutline) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "**Outline of `{}` ({})**: {} symbol{}, {} import{}\n",
        outline.file_path,
        outline.repo,
        outline.symbols.len(),
        plural(outline.symbols.len()),
        outline.imports.len(),
        plural(outline.imports.len()),
    );
    for symbol in &outline.symbols {
        let _ = write!(
            out,
            "- `{}` {} {}-{}",
            symbol.name,
            symbol.kind.as_str(),
            symbol.span.start_line,
            symbol.span.end_line
        );
        if symbol.is_exported {
            out.push_str(" [exported]");
        }
        out.push('\n');
    }
    let imports: Vec<String> = outline
        .imports
        .iter()
        .filter_map(|edge| edge.to_name.as_deref())
        .map(|name| format!("`{name}`"))
        .collect();
    if !imports.is_empty() {
        let _ = writeln!(out, "\nImports: {}", imports.join(", "));
    }
    out
}

/// Describes how far a reader should trust an edge with this provenance.
pub(super) fn provenance_word(p: Provenance) -> &'static str {
    match p {
        Provenance::Extracted => "read off the AST",
        Provenance::Inferred => "inferred, not certain",
        Provenance::Ambiguous => "ambiguous, several candidates",
    }
}

pub(super) fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/narrate/mod.rs"]
mod tests;
