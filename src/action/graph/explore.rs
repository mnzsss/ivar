//! Hero explore query engine synthesizing symbol discovery, surgical source code spans,
//! immediate call flows, and blast-radius impact analysis.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::action::graph::query::{self, QueryError};
use crate::domain::graph::{
    CallFlowItem, ExploreImpact, ExploreResult, OperationalRelation, RelationDirection,
    RelationEndpoint, SourceExcerpt, SourceFile, SymbolSnippet,
};
use crate::infra::hash;
use crate::store::graph::db::GraphDb;

/// Files up to this many lines are returned whole: an agent shown a slice of a
/// small file reads the whole file anyway, which costs more than sending it once.
const WHOLE_FILE_MAX_LINES: usize = 250;
const EXCERPT_MERGE_GAP: usize = 8;

/// Errors that can occur during explore synthesis.
#[derive(Debug, thiserror::Error)]
pub enum ExploreError {
    #[error("Database query failed: {0}")]
    Query(#[from] QueryError),
    #[error("Database error: {0}")]
    Db(#[from] crate::store::graph::db::GraphDbError),
    #[error("I/O error reading source file {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

struct FileSpans {
    repo: String,
    file_path: String,
    absolute_path: PathBuf,
    spans: Vec<(usize, usize)>,
}

struct CachedFile {
    lines: Vec<String>,
    content_hash: String,
}

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
        });
    }

    // Step 1: Match query against symbols via structured exploration retrieval pipeline
    // (path pinning, weighted OR terms, per-file limits).
    let candidates = query::find::explore_find_candidates(db, trimmed_query, repo)?;

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
        });
    }

    // Step 2: Fetch source snippets surgically
    let mut file_cache: HashMap<PathBuf, CachedFile> = HashMap::new();
    let mut primary_symbols = Vec::new();
    let mut file_spans: Vec<FileSpans> = Vec::new();

    for candidate in &candidates {
        let repo_root = if let Some(repo_row) = db.get_repo(&candidate.symbol.repo)? {
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

    // Step 3: Immediate call flows & operational relations (for primary symbols)
    let mut call_flows = Vec::new();
    let mut seen_flows = std::collections::HashSet::new();
    let mut direct_relations = Vec::new();
    let mut entry_points = Vec::new();
    let mut seen_relations = std::collections::HashSet::new();

    for candidate in &candidates {
        let candidate_endpoint = RelationEndpoint {
            repo: candidate.symbol.repo.clone(),
            file_path: candidate.file_path.clone(),
            symbol_name: candidate.symbol.name.clone(),
            symbol_kind: Some(candidate.symbol.kind.clone()),
        };

        // Query callers (incoming)
        let callers = query::get_callers(db, &candidate.symbol.name, repo, true, 0.7)?;
        for caller in callers {
            let flow_key = (
                caller.caller.name.clone(),
                candidate.symbol.name.clone(),
                caller.line,
            );
            if seen_flows.insert(flow_key) {
                call_flows.push(CallFlowItem {
                    caller: caller.caller.name.clone(),
                    callee: candidate.symbol.name.clone(),
                    edge_kind: caller.edge_kind.clone(),
                    provenance: caller.provenance,
                    line: caller.line,
                });
            }

            let caller_endpoint = RelationEndpoint {
                repo: caller.caller.repo.clone(),
                file_path: caller.caller_file_path.clone(),
                symbol_name: caller.caller.name.clone(),
                symbol_kind: Some(caller.caller.kind.clone()),
            };
            let is_cross_repo = caller.caller.repo != candidate.symbol.repo;
            let rel_key = (
                caller_endpoint.repo.clone(),
                caller_endpoint.file_path.clone(),
                caller_endpoint.symbol_name.clone(),
                candidate_endpoint.repo.clone(),
                candidate_endpoint.file_path.clone(),
                candidate_endpoint.symbol_name.clone(),
                caller.line,
                caller.edge_kind.clone(),
            );
            if seen_relations.insert(rel_key) {
                let rel = OperationalRelation {
                    source: caller_endpoint.clone(),
                    target: candidate_endpoint.clone(),
                    direction: RelationDirection::Incoming,
                    edge_kind: caller.edge_kind,
                    provenance: caller.provenance,
                    confidence: caller.confidence,
                    line: caller.line,
                    hop_count: 1,
                    cross_repo: is_cross_repo,
                };
                // Check if this incoming caller serves as an entry point (e.g. exported or CLI/root caller)
                if caller.caller.is_exported
                    || caller.caller_file_path.contains("cli")
                    || caller.caller_file_path.contains("main")
                {
                    entry_points.push(rel.clone());
                }
                direct_relations.push(rel);
            }
        }

        // Query callees (outgoing)
        if let Some(sym_id) = candidate.symbol.id {
            let callees = query::get_callees(db, sym_id)?;
            for callee in callees {
                let flow_key = (
                    candidate.symbol.name.clone(),
                    callee.callee_name.clone(),
                    callee.line,
                );
                if seen_flows.insert(flow_key) {
                    call_flows.push(CallFlowItem {
                        caller: candidate.symbol.name.clone(),
                        callee: callee.callee_name.clone(),
                        edge_kind: callee.edge_kind.clone(),
                        provenance: callee.provenance,
                        line: callee.line,
                    });
                }

                let callee_repo = callee
                    .callee_symbol
                    .as_ref()
                    .map(|s| s.repo.clone())
                    .unwrap_or_else(|| candidate.symbol.repo.clone());
                let callee_file_path = callee.callee_file_path.clone().unwrap_or_default();
                let callee_kind = callee.callee_symbol.as_ref().map(|s| s.kind.clone());

                let target_endpoint = RelationEndpoint {
                    repo: callee_repo.clone(),
                    file_path: callee_file_path.clone(),
                    symbol_name: callee.callee_name.clone(),
                    symbol_kind: callee_kind,
                };
                let is_cross_repo = callee_repo != candidate.symbol.repo;
                let rel_key = (
                    candidate_endpoint.repo.clone(),
                    candidate_endpoint.file_path.clone(),
                    candidate_endpoint.symbol_name.clone(),
                    target_endpoint.repo.clone(),
                    target_endpoint.file_path.clone(),
                    target_endpoint.symbol_name.clone(),
                    callee.line,
                    callee.edge_kind.clone(),
                );
                if seen_relations.insert(rel_key) {
                    direct_relations.push(OperationalRelation {
                        source: candidate_endpoint.clone(),
                        target: target_endpoint,
                        direction: RelationDirection::Outgoing,
                        edge_kind: callee.edge_kind,
                        provenance: callee.provenance,
                        confidence: callee.confidence,
                        line: callee.line,
                        hop_count: 1,
                        cross_repo: is_cross_repo,
                    });
                }
            }
        }
    }

    // Deterministic sorting for direct relations, entry points, and call flows
    direct_relations.sort_by(|a, b| {
        a.cross_repo
            .cmp(&b.cross_repo)
            .then_with(|| a.source.repo.cmp(&b.source.repo))
            .then_with(|| a.source.file_path.cmp(&b.source.file_path))
            .then_with(|| a.source.symbol_name.cmp(&b.source.symbol_name))
            .then_with(|| a.target.symbol_name.cmp(&b.target.symbol_name))
            .then_with(|| a.line.cmp(&b.line))
    });

    entry_points.sort_by(|a, b| {
        a.source
            .repo
            .cmp(&b.source.repo)
            .then_with(|| a.source.file_path.cmp(&b.source.file_path))
            .then_with(|| a.source.symbol_name.cmp(&b.source.symbol_name))
            .then_with(|| a.line.cmp(&b.line))
    });

    call_flows.sort_by(|a, b| {
        a.caller
            .cmp(&b.caller)
            .then_with(|| a.callee.cmp(&b.callee))
            .then_with(|| a.line.cmp(&b.line))
    });

    // Step 4: Blast radius / impact summary & transitive consumers
    let mut transitive_consumers = Vec::new();
    let impact_summary = if let Some(primary) = candidates.first() {
        if let Some(sym_id) = primary.symbol.id {
            let impact = query::get_impact(db, sym_id, 3)?;
            let total_callers = impact.total_affected;
            let total_files = impact.affected_files.len();

            for item in impact.affected_symbols {
                let is_cross_repo = item.symbol.repo != primary.symbol.repo;
                transitive_consumers.push(ExploreImpact {
                    symbol_name: item.symbol.name,
                    repo: item.symbol.repo,
                    file_path: item.file_path,
                    depth: item.depth,
                    path_via: item.path_via,
                    cross_repo: is_cross_repo,
                });
            }
            transitive_consumers.sort_by(|a, b| {
                a.depth
                    .cmp(&b.depth)
                    .then_with(|| a.repo.cmp(&b.repo))
                    .then_with(|| a.file_path.cmp(&b.file_path))
                    .then_with(|| a.symbol_name.cmp(&b.symbol_name))
            });

            if total_callers == 0 {
                Some(format!(
                    "Modifying '{}' has no known downstream callers.",
                    primary.symbol.name
                ))
            } else {
                let caller_str = if total_callers == 1 {
                    "1 caller"
                } else {
                    &format!("{total_callers} callers")
                };
                let file_str = if total_files == 1 {
                    "1 file"
                } else {
                    &format!("{total_files} files")
                };
                Some(format!(
                    "Modifying '{}' directly impacts {} across {}.",
                    primary.symbol.name, caller_str, file_str
                ))
            }
        } else {
            None
        }
    } else {
        None
    };

    Ok(ExploreResult {
        query: query.to_owned(),
        primary_symbols,
        call_flows,
        impact_summary,
        direct_relations,
        entry_points,
        transitive_consumers,
        sources,
    })
}

fn collect_sources(
    db: &GraphDb,
    cache: &HashMap<PathBuf, CachedFile>,
    files: Vec<FileSpans>,
) -> Result<Vec<SourceFile>, ExploreError> {
    let mut sources = Vec::with_capacity(files.len());
    for file in files {
        let Some(cached) = cache
            .get(&file.absolute_path)
            .filter(|cached| !cached.lines.is_empty())
        else {
            continue;
        };
        let lines = &cached.lines;
        // Spans come from the last index. Once the file changed they can cut a
        // function in half, so a changed file is served whole.
        let changed_since_index = db
            .get_file(&file.repo, &file.file_path)?
            .is_some_and(|row| row.content_hash != cached.content_hash);
        let ranges = if changed_since_index || lines.len() <= WHOLE_FILE_MAX_LINES {
            vec![(1, lines.len())]
        } else {
            merge_spans(file.spans)
        };
        let excerpts = ranges
            .into_iter()
            .map(|(start, end)| (start.max(1), end.min(lines.len())))
            .filter(|(start, end)| start <= end)
            .map(|(start, end)| SourceExcerpt {
                start_line: start,
                end_line: end,
                code: number_lines(lines, start, end),
            })
            .collect();
        sources.push(SourceFile {
            repo: file.repo,
            file_path: file.file_path,
            line_count: lines.len(),
            excerpts,
            changed_since_index,
        });
    }
    Ok(sources)
}

fn merge_spans(mut spans: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    spans.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    for (start, end) in spans {
        match merged.last_mut() {
            Some(last) if start <= last.1 + EXCERPT_MERGE_GAP + 1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// Reads source lines `[start_line, end_line]` (1-indexed, inclusive) and formats with line numbers.
fn get_source_snippet(
    cache: &mut HashMap<PathBuf, CachedFile>,
    file_path: &Path,
    start_line: usize,
    end_line: usize,
) -> Result<String, ExploreError> {
    let cached = match cache.get(file_path) {
        Some(cached) => cached,
        None => {
            let content = std::fs::read_to_string(file_path).map_err(|err| ExploreError::Io {
                path: file_path.to_path_buf(),
                source: err,
            })?;
            cache.entry(file_path.to_path_buf()).or_insert(CachedFile {
                lines: content.lines().map(str::to_owned).collect(),
                content_hash: hash::text(&content),
            })
        }
    };

    Ok(number_lines(&cached.lines, start_line, end_line))
}

fn number_lines(lines: &[String], start_line: usize, end_line: usize) -> String {
    let start = start_line.max(1);
    let end = end_line.max(start);
    (start..=end)
        .filter_map(|n| {
            n.checked_sub(1)
                .and_then(|idx| lines.get(idx))
                .map(|line| format!("{n}: {line}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/explore.rs"]
mod tests;
