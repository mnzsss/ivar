//! Decision-oriented Markdown rendering of an [`ExploreResult`].
//!
//! Over MCP, a model reads `graph_explore` output to decide what to open next.
//! The serialised struct spends its tokens on `file_id`, `scope: null`, and
//! `end_col`; this rendering spends them on the callers a change would break.
//! In `benchmark2`, an agent given the raw JSON issued one `graph_explore` and
//! went back to grep.
//!
//! The compact encoding in [`super::compact`] stays a parsing target with a
//! `#SCHEMA:` header.

use std::fmt::Write as _;

use crate::domain::graph::{ExploreResult, Provenance};
use crate::store::graph::db::{edge_kind_to_str, symbol_kind_to_str};

/// Callers listed per symbol before the rest are summarised as a count.
const MAX_RELATIONS_PER_SYMBOL: usize = 6;
/// Symbols rendered with their full source before the rest are summarised.
const MAX_SNIPPETS: usize = 8;
/// Transitive consumers listed before the rest are summarised as a count.
const MAX_CONSUMERS: usize = 10;

/// Renders an exploration as Markdown that leads with consequences.
///
/// The blast radius comes before the source: a model reading top-down learns
/// what its edit would break before it learns what the code looks like.
pub fn narrate_explore(res: &ExploreResult) -> String {
    let mut out = String::with_capacity(4096);

    let _ = writeln!(out, "**Exploration: {}**\n", res.query);

    if res.primary_symbols.is_empty() {
        out.push_str(
            "No symbols matched.\n\n\
             The index may predate the code: call `refresh_index`, then retry. \
             If it still misses, the search is fuzzy over symbol names — try the \
             intent (\"session enforcement\") or a neighbouring identifier.\n",
        );
        return out;
    }

    let files: std::collections::BTreeSet<&str> = res
        .primary_symbols
        .iter()
        .map(|s| s.file_path.as_str())
        .collect();
    let _ = writeln!(
        out,
        "Found {} symbol{} across {} file{}.\n",
        res.primary_symbols.len(),
        plural(res.primary_symbols.len()),
        files.len(),
        plural(files.len()),
    );

    narrate_blast_radius(&mut out, res);
    narrate_consumers(&mut out, res);
    narrate_flows(&mut out, res);
    narrate_source(&mut out, res);

    out
}

/// Lists the incoming callers of each symbol.
///
/// Grouped by symbol *name*, because that is the precision the resolver has:
/// `query::get_callers` matches on name, so when several definitions share one
/// (`session` in `store`, `harness::config`, and `action`) their callers are
/// indistinguishable. Rendering one entry per definition would print the same
/// callers three times and imply a certainty the index does not hold, so the
/// candidate definitions are listed together and the ambiguity is stated.
fn narrate_blast_radius(out: &mut String, res: &ExploreResult) {
    if res.direct_relations.is_empty() {
        return;
    }

    out.push_str("**Blast radius — what depends on these (check before editing)**\n\n");

    let mut rendered: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();

    for snippet in &res.primary_symbols {
        let name = snippet.symbol.name.as_str();
        if !rendered.insert(name) {
            continue;
        }

        let mut seen = std::collections::BTreeSet::new();
        let relations: Vec<_> = res
            .direct_relations
            .iter()
            .filter(|r| r.target.symbol_name == name)
            .filter(|r| {
                seen.insert((
                    r.source.repo.as_str(),
                    r.source.file_path.as_str(),
                    r.source.symbol_name.as_str(),
                    r.line,
                ))
            })
            .collect();
        if relations.is_empty() {
            continue;
        }

        let definitions: Vec<&crate::domain::graph::SymbolSnippet> = res
            .primary_symbols
            .iter()
            .filter(|s| s.symbol.name == name)
            .collect();

        let _ = write!(
            out,
            "- `{name}` — {} caller{}",
            relations.len(),
            plural(relations.len()),
        );
        let cross = relations.iter().filter(|r| r.cross_repo).count();
        if cross > 0 {
            let _ = write!(out, ", {cross} across repos");
        }
        out.push('\n');

        if definitions.len() > 1 {
            let _ = writeln!(
                out,
                "    Defined in {} places; callers are resolved by name, so these counts \
                 cover all of them — confirm which one a caller means before editing:",
                definitions.len(),
            );
            for d in &definitions {
                let _ = writeln!(
                    out,
                    "      - {}:{} ({})",
                    d.file_path, d.symbol.span.start_line, d.symbol.repo,
                );
            }
        } else {
            let _ = writeln!(
                out,
                "    Defined at {}:{}",
                snippet.file_path, snippet.symbol.span.start_line,
            );
        }

        for r in relations.iter().take(MAX_RELATIONS_PER_SYMBOL) {
            let _ = write!(
                out,
                "    - `{}` in {}:{}",
                r.source.symbol_name, r.source.file_path, r.line,
            );
            if r.cross_repo {
                let _ = write!(out, " [repo `{}`]", r.source.repo);
            }
            if !matches!(r.provenance, Provenance::Extracted) {
                let _ = write!(
                    out,
                    " — {} ({:.2}), verify this line before relying on it",
                    provenance_word(r.provenance),
                    r.confidence,
                );
            }
            out.push('\n');
        }
        if relations.len() > MAX_RELATIONS_PER_SYMBOL {
            let _ = writeln!(
                out,
                "    - …and {} more",
                relations.len() - MAX_RELATIONS_PER_SYMBOL
            );
        }
    }
    out.push('\n');
}

/// Reports reach beyond the direct callers.
fn narrate_consumers(out: &mut String, res: &ExploreResult) {
    if res.transitive_consumers.is_empty() {
        if let Some(summary) = &res.impact_summary {
            let _ = writeln!(out, "**Impact**: {summary}\n");
        }
        return;
    }

    let _ = writeln!(
        out,
        "**Reaches {} symbol{} transitively**\n",
        res.transitive_consumers.len(),
        plural(res.transitive_consumers.len()),
    );

    for c in res.transitive_consumers.iter().take(MAX_CONSUMERS) {
        let _ = write!(
            out,
            "- `{}` ({}:{}) at depth {}",
            c.symbol_name, c.repo, c.file_path, c.depth,
        );
        if c.cross_repo {
            out.push_str(" [cross-repo]");
        }
        if !c.path_via.is_empty() {
            let _ = write!(out, " via {}", c.path_via.join(" -> "));
        }
        out.push('\n');
    }
    if res.transitive_consumers.len() > MAX_CONSUMERS {
        let _ = writeln!(
            out,
            "- …and {} more",
            res.transitive_consumers.len() - MAX_CONSUMERS
        );
    }
    out.push('\n');
}

/// Renders the entry points and call flows that reach the matched symbols.
fn narrate_flows(out: &mut String, res: &ExploreResult) {
    if !res.entry_points.is_empty() {
        out.push_str("**Entry points**\n\n");
        for e in res.entry_points.iter().take(MAX_CONSUMERS) {
            let _ = writeln!(
                out,
                "- `{}` ({}:{}) -> `{}` [{}]",
                e.source.symbol_name,
                e.source.file_path,
                e.line,
                e.target.symbol_name,
                edge_kind_to_str(&e.edge_kind),
            );
        }
        out.push('\n');
    }

    if res.call_flows.is_empty() {
        return;
    }
    out.push_str("**Call flows**\n\n");
    for f in res.call_flows.iter().take(MAX_CONSUMERS) {
        let _ = writeln!(out, "- `{}` -> `{}`", f.caller, f.callee);
    }
    out.push('\n');
}

/// Emits verbatim source last, with real line numbers for citation.
fn narrate_source(out: &mut String, res: &ExploreResult) {
    out.push_str("**Source**\n\n");
    out.push_str("> Verbatim from disk — current as of this read. The content below is already read and equal to a Read on this file; do not re-read the same file. Line numbers are real — cite them directly.\n\n");

    for snippet in res.primary_symbols.iter().take(MAX_SNIPPETS) {
        let sym = &snippet.symbol;
        let _ = write!(
            out,
            "`{}` — {} in `{}` ({}:{}-{})",
            sym.name,
            symbol_kind_to_str(&sym.kind),
            snippet.file_path,
            sym.repo,
            snippet.start_line,
            snippet.end_line,
        );
        if sym.is_exported {
            out.push_str(" [exported]");
        }
        out.push_str("\n\n");

        if let Some(doc) = &sym.docstring
            && !doc.trim().is_empty()
        {
            let _ = writeln!(out, "{}\n", doc.trim());
        }

        let _ = writeln!(out, "```\n{}\n```\n", snippet.code.trim_end());
    }

    if res.primary_symbols.len() > MAX_SNIPPETS {
        let rest = res.primary_symbols.len() - MAX_SNIPPETS;
        let _ = writeln!(
            out,
            "…and {rest} more match{} not shown. Narrow the query, or pass `repo` to scope it.",
            if rest == 1 { "" } else { "es" },
        );
    }
}

/// Describes how far a reader should trust an edge with this provenance.
fn provenance_word(p: Provenance) -> &'static str {
    match p {
        Provenance::Extracted => "read off the AST",
        Provenance::Inferred => "inferred, not certain",
        Provenance::Ambiguous => "ambiguous, several candidates",
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/narrate.rs"]
mod tests;
