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

use std::fmt::Write as _;

use crate::action::graph::query::{
    CalleeInfo, CallerInfo, FileOutline, ImpactResult, ReferenceSite, SymbolLocation,
};
use crate::domain::graph::{EdgeKind, ExploreResult, Provenance, SourceFile, SymbolSnippet};
use crate::store::graph::db::edge_kind_to_str;

/// Callers listed per symbol before the rest are summarised as a count.
const MAX_RELATIONS_PER_SYMBOL: usize = 6;
/// Transitive consumers listed before the rest are summarised as a count.
const MAX_CONSUMERS: usize = 10;
/// Entries a callers, callees or impact answer lists before counting the rest.
const MAX_LIST_ITEMS: usize = 40;
/// Hosts save a tool result longer than about 25K characters to a file the agent
/// then has to Read, which costs more tokens than the answer saves.
const MAX_OUTPUT_CHARS: usize = 24_000;
/// Source keeps this much room even after long relation sections.
const MIN_SOURCE_CHARS: usize = 8_000;

struct FileMatches<'a> {
    repo: &'a str,
    path: &'a str,
    symbols: Vec<&'a SymbolSnippet>,
}

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

    let files = files_in_rank_order(res);
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
    narrate_source(&mut out, res, &files);

    out
}

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

/// Groups matched symbols by file in the order explore ranked them.
fn files_in_rank_order(res: &ExploreResult) -> Vec<FileMatches<'_>> {
    let mut files: Vec<FileMatches<'_>> = Vec::new();
    for snippet in &res.primary_symbols {
        match files
            .iter_mut()
            .find(|f| f.repo == snippet.symbol.repo && f.path == snippet.file_path)
        {
            Some(file) => file.symbols.push(snippet),
            None => files.push(FileMatches {
                repo: &snippet.symbol.repo,
                path: &snippet.file_path,
                symbols: vec![snippet],
            }),
        }
    }
    files
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
    let has_callers = res.primary_symbols.iter().any(|snippet| {
        res.direct_relations
            .iter()
            .any(|r| r.target.symbol_name == snippet.symbol.name)
    });
    if !has_callers {
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

        let definitions: Vec<&SymbolSnippet> = res
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

    let leaves_index: std::collections::BTreeSet<(&str, &str)> = res
        .direct_relations
        .iter()
        .filter(|r| {
            matches!(
                r.direction,
                crate::domain::graph::RelationDirection::Outgoing
            ) && r.target.file_path.is_empty()
        })
        .map(|r| (r.source.symbol_name.as_str(), r.target.symbol_name.as_str()))
        .collect();
    let flows: Vec<_> = res
        .call_flows
        .iter()
        .filter(|f| !leaves_index.contains(&(f.caller.as_str(), f.callee.as_str())))
        .collect();
    if flows.is_empty() {
        return;
    }
    out.push_str("**Call flows**\n\n");
    for f in flows.iter().take(MAX_CONSUMERS) {
        let _ = writeln!(out, "- `{}` -> `{}`", f.caller, f.callee);
    }
    out.push('\n');
}

/// Emits verbatim source last, one block per file, within the output budget.
fn narrate_source(out: &mut String, res: &ExploreResult, files: &[FileMatches<'_>]) {
    out.push_str("**Source**\n\n");
    out.push_str("> Verbatim from disk — current as of this read. The content below is already read and equal to a Read on this file; do not re-read the same file. Line numbers are real — cite them directly.\n\n");

    let limit = MAX_OUTPUT_CHARS.max(out.len() + MIN_SOURCE_CHARS);
    let mut emitted = false;
    let mut left_out = Vec::new();
    for file in files {
        let source = res
            .sources
            .iter()
            .find(|s| s.repo == file.repo && s.file_path == file.path);
        let block = file_block(file, source);
        if out.len() + block.len() <= limit {
            out.push_str(&block);
            emitted = true;
        } else if !emitted {
            out.push_str(&truncate_block(
                &block,
                limit.saturating_sub(out.len() + 160),
            ));
            emitted = true;
        } else {
            left_out.push(format!("`{}`", file.path));
        }
    }

    if !left_out.is_empty() {
        let _ = writeln!(
            out,
            "Not shown, to keep this answer under the size limit: {}. Call `graph_explore` with \
             those paths to see their source.",
            left_out.join(", ")
        );
    }
}

fn file_block(file: &FileMatches<'_>, source: Option<&SourceFile>) -> String {
    let mut block = String::new();
    let _ = write!(block, "`{}` ({}", file.path, file.repo);
    match source {
        Some(source) if source.changed_since_index => {
            let _ = write!(
                block,
                " · whole file, {} line{}, changed since the last index so the symbol lines \
                 below may have moved",
                source.line_count,
                plural(source.line_count)
            );
        }
        Some(source) if is_whole_file(source) => {
            let _ = write!(
                block,
                " · whole file, {} line{}",
                source.line_count,
                plural(source.line_count)
            );
        }
        Some(source) => {
            let ranges: Vec<String> = source
                .excerpts
                .iter()
                .map(|e| format!("{}-{}", e.start_line, e.end_line))
                .collect();
            let _ = write!(
                block,
                " · lines {} of {}",
                ranges.join(", "),
                source.line_count
            );
        }
        None => block.push_str(" · source unavailable"),
    }
    let names: Vec<String> = file
        .symbols
        .iter()
        .map(|s| format!("`{}` {}", s.symbol.name, s.symbol.span.start_line))
        .collect();
    let _ = writeln!(block, ") · {}\n", names.join(", "));

    for excerpt in source.map(|s| s.excerpts.as_slice()).unwrap_or_default() {
        let _ = writeln!(block, "```\n{}\n```\n", excerpt.code.trim_end());
    }
    block
}

fn is_whole_file(source: &SourceFile) -> bool {
    matches!(source.excerpts.as_slice(), [only] if only.start_line == 1 && only.end_line == source.line_count)
}

fn truncate_block(block: &str, max_len: usize) -> String {
    let mut cut = String::new();
    let mut last_line = None;
    for line in block.lines() {
        if cut.len() + line.len() + 1 > max_len {
            break;
        }
        cut.push_str(line);
        cut.push('\n');
        if let Some(n) = line
            .split_once(": ")
            .and_then(|(n, _)| n.parse::<usize>().ok())
        {
            last_line = Some(n);
        }
    }
    if cut.matches("```").count() % 2 == 1 {
        cut.push_str("```\n");
    }
    if let Some(n) = last_line {
        let _ = writeln!(
            cut,
            "\n…truncated after line {n}. Call `graph_explore` with a narrower query to see the rest."
        );
    }
    cut
}

fn push_remainder(out: &mut String, total: usize) {
    if total > MAX_LIST_ITEMS {
        let _ = writeln!(out, "- …and {} more", total - MAX_LIST_ITEMS);
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
