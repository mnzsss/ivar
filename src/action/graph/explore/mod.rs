//! Hero explore query engine synthesizing symbol discovery, surgical source code spans,
//! immediate call flows, and blast-radius impact analysis.

pub mod error;
pub mod flow;
pub mod impact;
pub mod source;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::action::graph::query;
use crate::domain::graph::{ExploreResult, SymbolSnippet};
use crate::store::graph::db::GraphDb;

pub use error::ExploreError;

use self::flow::named_flows;
use self::impact::{collect_impact, collect_relations};
use self::source::{CachedFile, FileSpans, collect_sources, get_source_snippet, max_source_files};

/// Explores the codebase graph for a given query string, returning matched symbols with
/// verbatim code snippets, immediate call flows, and blast-radius impact.
pub fn explore(
    db: &GraphDb,
    hall_root: &Path,
    query: &str,
    repo: Option<&str>,
) -> Result<ExploreResult, ExploreError> {
    let trimmed_query = query.trim();
    if trimmed_query.is_empty() {
        return Ok(ExploreResult {
            query: query.to_owned(),
            primary_symbols: Vec::new(),
            call_flows: Vec::new(),
            impact_summary: None,
            direct_relations: Vec::new(),
            entry_points: Vec::new(),
            transitive_consumers: Vec::new(),
            sources: Vec::new(),
            flows: Vec::new(),
            not_shown: Vec::new(),
        });
    }

    // Step 1: Match query against symbols via structured exploration retrieval pipeline
    // (path pinning, weighted OR terms, per-file limits).
    let found = query::find::explore_find(db, trimmed_query, repo, max_source_files(db)?)?;
    let candidates = found.symbols;
    let not_shown = found.not_shown;

    if candidates.is_empty() {
        return Ok(ExploreResult {
            query: query.to_owned(),
            primary_symbols: Vec::new(),
            call_flows: Vec::new(),
            impact_summary: Some(format!("No symbols found matching query '{query}'.")),
            direct_relations: Vec::new(),
            entry_points: Vec::new(),
            transitive_consumers: Vec::new(),
            sources: Vec::new(),
            flows: Vec::new(),
            not_shown: Vec::new(),
        });
    }

    // Step 2: Fetch source snippets surgically
    let mut file_cache: HashMap<PathBuf, CachedFile> = HashMap::new();
    let mut primary_symbols = Vec::new();
    let mut file_spans: Vec<FileSpans> = Vec::new();

    for candidate in &candidates {
        let repo_root = if let Some(repo_row) = db.get_visible_repo(&candidate.symbol.repo)? {
            PathBuf::from(repo_row.root_path)
        } else {
            hall_root.join(&candidate.symbol.repo)
        };

        let file_path = repo_root.join(&candidate.file_path);
        let start_line = candidate.symbol.span.start_line;
        let end_line = candidate.symbol.span.end_line;

        let code = match get_source_snippet(&mut file_cache, &file_path, start_line, end_line) {
            Ok(snippet) => snippet,
            Err(e) => {
                // If file cannot be read, format error or fallback gracefully
                format!("<failed to read source: {e}>")
            }
        };

        match file_spans
            .iter_mut()
            .find(|f| f.repo == candidate.symbol.repo && f.file_path == candidate.file_path)
        {
            Some(entry) => entry.spans.push((start_line, end_line)),
            None => file_spans.push(FileSpans {
                repo: candidate.symbol.repo.clone(),
                file_path: candidate.file_path.clone(),
                absolute_path: file_path.clone(),
                spans: vec![(start_line, end_line)],
            }),
        }

        primary_symbols.push(SymbolSnippet {
            symbol: candidate.symbol.clone(),
            file_path: candidate.file_path.clone(),
            code,
            start_line,
            end_line,
        });
    }

    let sources = collect_sources(db, &file_cache, file_spans)?;
    let flows = named_flows(db, trimmed_query, &candidates)?;

    // Step 3: Immediate call flows & operational relations (for primary symbols)
    let relations = collect_relations(db, &candidates, repo)?;

    // Step 4: Blast radius / impact summary & transitive consumers
    let (impact_summary, transitive_consumers) = collect_impact(db, &candidates)?;

    Ok(ExploreResult {
        query: query.to_owned(),
        primary_symbols,
        call_flows: relations.call_flows,
        impact_summary,
        direct_relations: relations.direct_relations,
        entry_points: relations.entry_points,
        transitive_consumers,
        sources,
        flows,
        not_shown,
    })
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/explore.rs"]
mod tests;
