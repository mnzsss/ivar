use super::source::{narrate_next_call, narrate_not_shown, narrate_source};
use super::*;

pub(super) struct FileMatches<'a> {
    pub(super) repo: &'a str,
    pub(super) path: &'a str,
    pub(super) symbols: Vec<&'a SymbolSnippet>,
    pub(super) file_matches: Vec<&'a FileMatch>,
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

    if res.confidence == ExploreConfidence::NoMatch {
        narrate_no_match(&mut out, res);
        return out;
    }

    let files = files_in_rank_order(res);
    let symbols = res.primary_symbols.iter().filter(|s| !s.weak).count();
    let file_matches = res.file_matches.iter().filter(|f| !f.weak).count();
    if symbols > 0 {
        let _ = writeln!(
            out,
            "Found {symbols} symbol{} across {} file{}.\n",
            plural(symbols),
            files.len(),
            plural(files.len()),
        );
    } else {
        let _ = writeln!(
            out,
            "Found {file_matches} file match{} across {} file{}.\n",
            plural(file_matches),
            files.len(),
            plural(files.len()),
        );
    }
    if res.confidence == ExploreConfidence::Partial {
        out.push_str(
            "Confidence: partial. Some candidates cover too little of the query; they are \
             listed last, weak and without source.\n\n",
        );
    }

    narrate_named_flows(&mut out, res);
    narrate_blast_radius(&mut out, res);

    if is_list_intent(&res.query) && symbols == 0 {
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
        narrate_weak(&mut out, res);
        return out;
    }

    let left_out = narrate_source(&mut out, res, &files, max_chars);
    narrate_not_shown(&mut out, &left_out, &res.not_shown);
    narrate_weak(&mut out, res);

    out
}

/// Answers a query no candidate covers: says so, and lists the closest
/// candidates as locations only, so the agent does not read them as an answer.
fn narrate_no_match(out: &mut String, res: &ExploreResult) {
    let _ = writeln!(
        out,
        "Confidence: none. No confident match for '{}'.\n\n\
         No indexed file covers enough of the query's words. The candidates below are weak \
         guesses, listed as locations without source: do not treat them as the answer. Search \
         with a name or path from the topic, or fall back to a text search.\n",
        res.query
    );
    narrate_weak(out, res);
}

/// Lists the weak candidates as `path:line`, after the strong ones.
fn narrate_weak(out: &mut String, res: &ExploreResult) {
    let mut locations: Vec<String> = Vec::new();
    for fm in res.file_matches.iter().filter(|f| f.weak) {
        locations.push(format!("{}:{}", fm.file_path, fm.start_line));
    }
    for snippet in res.primary_symbols.iter().filter(|s| s.weak) {
        locations.push(format!(
            "{}:{} `{}`",
            snippet.file_path, snippet.start_line, snippet.symbol.name
        ));
    }
    if locations.is_empty() {
        return;
    }
    out.push_str("**Weak matches (below the relevance threshold, no source)**\n\n");
    for location in locations.iter().take(MAX_WEAK_LISTED) {
        let _ = writeln!(out, "- {location}");
    }
    if locations.len() > MAX_WEAK_LISTED {
        let _ = writeln!(out, "- …and {} more", locations.len() - MAX_WEAK_LISTED);
    }
}

/// Groups matched symbols and file matches by file in the order explore ranked them.
fn files_in_rank_order(res: &ExploreResult) -> Vec<FileMatches<'_>> {
    let mut files: Vec<FileMatches<'_>> = Vec::new();
    for snippet in res.primary_symbols.iter().filter(|s| !s.weak) {
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
    for fm in res.file_matches.iter().filter(|f| !f.weak) {
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
