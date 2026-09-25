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

use crate::action::graph::explore::MAX_REQUESTED_FILES;
use crate::action::graph::query::{
    CalleeInfo, CallerInfo, FileOutline, ImpactResult, ReferenceSite, SymbolLocation,
};
use crate::domain::graph::{
    EdgeKind, ExploreResult, FileMatch, FileMention, MentionedSymbol, Provenance, SourceFile,
    SymbolSnippet,
};

/// Definitions listed for a symbol before the rest are left out.
const MAX_RELATIONS_PER_SYMBOL: usize = 6;
/// Symbols the blast radius names before counting the rest.
const MAX_BLAST_SYMBOLS: usize = 8;
/// Caller files named on one blast-radius line.
const MAX_CALLER_FILES: usize = 3;
/// Files the "not shown" list names before counting the rest.
const MAX_NOT_SHOWN_FILES: usize = 10;
/// Symbol names listed for one file that got no source.
const MAX_NAMES_PER_FILE: usize = 6;
/// Entries a callers, callees or impact answer lists before counting the rest.
const MAX_LIST_ITEMS: usize = 40;
/// Every later turn repeats an answer, so it stays well under the ~25K characters
/// at which hosts save a tool result to a file the agent then has to Read.
const MAX_OUTPUT_CHARS: usize = 18_000;
/// A `paths` answer may run longer, still under the ~25K-character threshold.
const MAX_REQUESTED_OUTPUT_CHARS: usize = 24_000;
/// Room kept after the source for the not-shown list and the next call, so no
/// answer passes [`MAX_REQUESTED_OUTPUT_CHARS`].
const NOT_SHOWN_RESERVE_CHARS: usize = 4_000;
/// Source keeps this much room even after long relation sections.
const MIN_SOURCE_CHARS: usize = 8_000;

struct FileMatches<'a> {
    repo: &'a str,
    path: &'a str,
    symbols: Vec<&'a SymbolSnippet>,
    file_matches: Vec<&'a FileMatch>,
}

fn is_list_intent(query: &str) -> bool {
    let q = query.to_ascii_lowercase();
    q.contains("importer")
        || q.contains("importers")
        || q.contains("caller")
        || q.contains("callers")
        || q.contains("test file")
        || q.contains("test files")
        || q.contains("list files")
        || q.contains("files importing")
}
/// Renders an exploration as Markdown that leads with consequences.
///
/// The blast radius comes before the source: a model reading top-down learns
/// what its edit would break before it learns what the code looks like.
pub fn narrate_explore(res: &ExploreResult) -> String {
    narrate_explore_within(res, MAX_OUTPUT_CHARS)
}

/// Renders an exploration of files the agent named, with the larger budget of
/// [`crate::action::graph::explore::explore_files`].
pub fn narrate_requested_files(res: &ExploreResult) -> String {
    let mut out = narrate_explore_within(res, MAX_REQUESTED_OUTPUT_CHARS);
    if out.contains("\nNext: call `graph_explore`") {
        let _ = writeln!(
            out,
            "One call returns at most {MAX_REQUESTED_FILES} files and \
             {MAX_REQUESTED_OUTPUT_CHARS} characters."
        );
    }
    out
}

fn narrate_explore_within(res: &ExploreResult, max_chars: usize) -> String {
    let mut out = String::with_capacity(4096);

    let _ = writeln!(out, "**Exploration: {}**\n", res.query);

    if res.primary_symbols.is_empty() && res.file_matches.is_empty() {
        out.push_str(
            "No symbols matched.\n\n\
             The index may predate the code: call `refresh_index`, then retry. \
             If it still misses, the search is fuzzy over symbol names — try the \
             intent (\"session enforcement\") or a neighbouring identifier.\n",
        );
        return out;
    }

    let files = files_in_rank_order(res);
    if !res.primary_symbols.is_empty() {
        let _ = writeln!(
            out,
            "Found {} symbol{} across {} file{}.\n",
            res.primary_symbols.len(),
            plural(res.primary_symbols.len()),
            files.len(),
            plural(files.len()),
        );
    } else {
        let _ = writeln!(
            out,
            "Found {} file match{} across {} file{}.\n",
            res.file_matches.len(),
            plural(res.file_matches.len()),
            files.len(),
            plural(files.len()),
        );
    }

    narrate_named_flows(&mut out, res);
    narrate_blast_radius(&mut out, res);

    if is_list_intent(&res.query) && res.primary_symbols.is_empty() {
        out.push_str("**Matching Files**\n\n");
        for file in &files {
            let _ = writeln!(out, "- `{}` ({})", file.path, file.repo);
        }
        let file_mentions: Vec<FileMention> = files
            .iter()
            .map(|f| FileMention {
                repo: f.repo.to_owned(),
                file_path: f.path.to_owned(),
                symbols: f
                    .symbols
                    .iter()
                    .map(|s| MentionedSymbol {
                        name: s.symbol.name.clone(),
                        line: s.symbol.span.start_line,
                    })
                    .collect(),
            })
            .collect();
        let mentions_refs: Vec<&FileMention> = file_mentions.iter().collect();
        narrate_next_call(&mut out, &mentions_refs);
        return out;
    }

    let left_out = narrate_source(&mut out, res, &files, max_chars);
    narrate_not_shown(&mut out, &left_out, &res.not_shown);

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

/// Groups matched symbols and file matches by file in the order explore ranked them.
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
                file_matches: Vec::new(),
            }),
        }
    }
    for fm in &res.file_matches {
        match files
            .iter_mut()
            .find(|f| f.repo == fm.repo && f.path == fm.file_path)
        {
            Some(file) => file.file_matches.push(fm),
            None => files.push(FileMatches {
                repo: &fm.repo,
                path: &fm.file_path,
                symbols: Vec::new(),
                file_matches: vec![fm],
            }),
        }
    }
    files
}
/// Leads with the path between the symbols a query named together.
fn narrate_named_flows(out: &mut String, res: &ExploreResult) {
    if res.flows.is_empty() {
        return;
    }
    out.push_str("**Flow between the symbols you named**\n\n");
    for flow in &res.flows {
        let _ = write!(out, "- `{}`", flow.from);
        for step in &flow.steps {
            let _ = write!(
                out,
                " → `{}` ({} at line {})",
                step.target,
                step.edge_kind.as_str(),
                step.line
            );
        }
        out.push('\n');
    }
    out.push('\n');
}

/// Names what depends on each matched symbol, one line per definition: how many
/// callers, in which files, and where its type is used outside any symbol body.
///
/// A symbol with an index id owns only the relations whose target is its exact
/// repo, file and name, so same-named definitions keep their consumers apart.
/// A symbol without an id falls back to grouping by name, the only precision the
/// name-based resolver has.
fn blast_radius_line_for_symbol(
    res: &ExploreResult,
    snippet: &SymbolSnippet,
    rendered: &mut std::collections::BTreeSet<(Option<(String, String)>, String)>,
) -> Option<String> {
    let name = snippet.symbol.name.as_str();
    let exact = snippet.symbol.id.is_some();
    let key = if exact {
        (
            Some((snippet.symbol.repo.clone(), snippet.file_path.clone())),
            name.to_owned(),
        )
    } else {
        (None, name.to_owned())
    };
    if !rendered.insert(key) {
        return None;
    }

    let mut seen = std::collections::BTreeSet::new();
    let relations: Vec<_> = res
        .direct_relations
        .iter()
        .filter(|r| r.target.symbol_name == name)
        .filter(|r| {
            !exact
                || (r.target.repo == snippet.symbol.repo && r.target.file_path == snippet.file_path)
        })
        .filter(|r| {
            seen.insert((
                r.source.repo.as_str(),
                r.source.file_path.as_str(),
                r.source.symbol_name.as_str(),
                r.line,
            ))
        })
        .collect();
    let (type_uses, callers): (Vec<_>, Vec<_>) = relations.into_iter().partition(|r| {
        exact && r.edge_kind == EdgeKind::References && r.source.symbol_name.is_empty()
    });
    if callers.is_empty() && type_uses.is_empty() {
        return None;
    }

    let files: std::collections::BTreeSet<&str> = callers
        .iter()
        .map(|r| r.source.file_path.as_str())
        .collect();
    let named: Vec<String> = files
        .iter()
        .take(MAX_CALLER_FILES)
        .map(|file| format!("`{file}`"))
        .collect();
    let mut line = format!(
        "- `{name}` ({}:{}) — {} caller{}",
        snippet.file_path,
        snippet.symbol.span.start_line,
        callers.len(),
        plural(callers.len()),
    );
    if !named.is_empty() {
        let _ = write!(line, " in {}", named.join(", "));
    }
    if files.len() > MAX_CALLER_FILES {
        let _ = write!(line, " +{} more files", files.len() - MAX_CALLER_FILES);
    }
    if !type_uses.is_empty() {
        let sites: Vec<String> = type_uses
            .iter()
            .map(|r| format!("`{}:{}`", r.source.file_path, r.line))
            .collect();
        let _ = write!(
            line,
            "; {} type use{} at {}",
            type_uses.len(),
            plural(type_uses.len()),
            sites.join(", ")
        );
    }
    let cross = callers.iter().filter(|r| r.cross_repo).count();
    if cross > 0 {
        let _ = write!(line, "; {cross} from other repos");
    }
    let definitions = res
        .primary_symbols
        .iter()
        .filter(|s| s.symbol.name == name)
        .count();
    if !exact && definitions > 1 {
        let _ = write!(
            line,
            "; {definitions} definitions share this name, so confirm which one a caller means"
        );
    }
    let uncertain = callers
        .iter()
        .filter(|r| !matches!(r.provenance, Provenance::Extracted))
        .count();
    if uncertain > 0 {
        let _ = write!(
            line,
            "; {uncertain} {}, verify before relying on them",
            provenance_word(Provenance::Inferred)
        );
    }
    Some(line)
}

fn narrate_blast_radius(out: &mut String, res: &ExploreResult) {
    let mut rendered = std::collections::BTreeSet::new();
    let lines: Vec<String> = res
        .primary_symbols
        .iter()
        .filter_map(|snippet| blast_radius_line_for_symbol(res, snippet, &mut rendered))
        .collect();

    if lines.is_empty() {
        return;
    }
    out.push_str("**Blast radius — what depends on these (check before editing)**\n\n");
    for line in lines.iter().take(MAX_BLAST_SYMBOLS) {
        let _ = writeln!(out, "{line}");
    }
    if lines.len() > MAX_BLAST_SYMBOLS {
        let _ = writeln!(
            out,
            "- …and {} more symbols with callers",
            lines.len() - MAX_BLAST_SYMBOLS
        );
    }
    out.push('\n');
}

/// Emits verbatim source, one block per file, within the output budget, and
/// returns the files that did not fit.
fn narrate_source(
    out: &mut String,
    res: &ExploreResult,
    files: &[FileMatches<'_>],
    max_chars: usize,
) -> Vec<FileMention> {
    out.push_str("**Source**\n\n");
    out.push_str(
        "> The code below is the verbatim, current on-disk source, line-numbered like the Read \
         tool. Treat each block as a Read you have already performed: do not Read a file shown \
         here.\n\n",
    );

    let limit = max_chars
        .max(out.len() + MIN_SOURCE_CHARS)
        .min(MAX_REQUESTED_OUTPUT_CHARS - NOT_SHOWN_RESERVE_CHARS);
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
            left_out.push(FileMention {
                repo: file.repo.to_owned(),
                file_path: file.path.to_owned(),
                symbols: file
                    .symbols
                    .iter()
                    .map(|s| MentionedSymbol {
                        name: s.symbol.name.clone(),
                        line: s.symbol.span.start_line,
                    })
                    .collect(),
            });
        }
    }
    left_out
}

/// Names the matching files that got no source, so the next step is another
/// `graph_explore` with those names instead of a Read.
fn narrate_not_shown(out: &mut String, left_out: &[FileMention], not_shown: &[FileMention]) {
    let files: Vec<&FileMention> = left_out.iter().chain(not_shown).collect();
    if files.is_empty() {
        return;
    }
    out.push_str(
        "**Not shown above — call `graph_explore` with these paths or names for their source**\n\n",
    );
    for file in files.iter().take(MAX_NOT_SHOWN_FILES) {
        let names: Vec<String> = file
            .symbols
            .iter()
            .take(MAX_NAMES_PER_FILE)
            .map(|s| format!("{}:{}", s.name, s.line))
            .collect();
        let _ = write!(out, "- {}: {}", file.file_path, names.join(", "));
        if file.symbols.len() > MAX_NAMES_PER_FILE {
            let _ = write!(out, ", +{} more", file.symbols.len() - MAX_NAMES_PER_FILE);
        }
        out.push('\n');
    }
    if files.len() > MAX_NOT_SHOWN_FILES {
        let _ = writeln!(
            out,
            "- …and {} more files",
            files.len() - MAX_NOT_SHOWN_FILES
        );
    }
    narrate_next_call(out, &files);
}

/// Agents Read left-out files one per turn, each turn repeating the whole
/// prompt, unless the answer spells out the single call that returns them.
fn narrate_next_call(out: &mut String, files: &[&FileMention]) {
    let batch: Vec<&str> = files
        .iter()
        .take(MAX_REQUESTED_FILES)
        .map(|file| file.file_path.as_str())
        .collect();
    let args = serde_json::json!({ "paths": batch });
    let _ = write!(
        out,
        "\nNext: call `graph_explore` with {args} for their full source, one call instead of {} Reads.",
        batch.len()
    );
    let rest = files.len() - batch.len();
    if rest > 0 {
        let _ = write!(out, " Then call it again for the {rest} files after them.");
    }
    out.push('\n');
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
    if !names.is_empty() {
        let _ = writeln!(block, ") · {}\n", names.join(", "));
    } else if !file.file_matches.is_empty() {
        let _ = writeln!(block, ") · matched file\n");
    } else {
        let _ = writeln!(block, ")\n");
    }

    let language = fence_language(file.path);
    for excerpt in source.map(|s| s.excerpts.as_slice()).unwrap_or_default() {
        let _ = writeln!(block, "```{language}\n{}\n```\n", excerpt.code.trim_end());
    }
    block
}

fn fence_language(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("ts" | "mts" | "cts") => "typescript",
        Some("tsx") => "tsx",
        Some("js" | "mjs" | "cjs" | "jsx") => "javascript",
        Some("rs") => "rust",
        _ => "",
    }
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
            .split_once('\t')
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
            "\n…truncated after line {n}. Call `graph_explore` with the names in this file for \
             the rest; do not Read it."
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
