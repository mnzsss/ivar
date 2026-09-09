//! Hero explore query engine synthesizing symbol discovery, surgical source code spans,
//! immediate call flows, and blast-radius impact analysis.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::action::graph::query::{self, QueryError};
use crate::domain::graph::{
    CallFlowItem, ExploreImpact, ExploreResult, OperationalRelation, RelationDirection,
    RelationEndpoint, SymbolSnippet,
};
use crate::store::graph::db::GraphDb;

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
        });
    }

    // Step 1: Match query against symbols
    let mut candidates = query::find_symbols(db, trimmed_query, repo, 5)?;

    // If no direct matches, try fallback to individual words in the query
    if candidates.is_empty() {
        for word in trimmed_query.split_whitespace() {
            let word_clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if word_clean.len() >= 3 {
                let word_candidates = query::find_symbols(db, word_clean, repo, 5)?;
                for c in word_candidates {
                    if !candidates
                        .iter()
                        .any(|existing| existing.symbol.id == c.symbol.id)
                    {
                        candidates.push(c);
                        if candidates.len() >= 5 {
                            break;
                        }
                    }
                }
            }
            if candidates.len() >= 5 {
                break;
            }
        }
    }

    if candidates.is_empty() {
        return Ok(ExploreResult {
            query: query.to_owned(),
            primary_symbols: Vec::new(),
            call_flows: Vec::new(),
            impact_summary: Some(format!("No symbols found matching query '{query}'.")),
            direct_relations: Vec::new(),
            entry_points: Vec::new(),
            transitive_consumers: Vec::new(),
        });
    }

    // Step 2: Fetch source snippets surgically
    let mut file_cache: HashMap<PathBuf, Vec<String>> = HashMap::new();
    let mut primary_symbols = Vec::new();

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

        primary_symbols.push(SymbolSnippet {
            symbol: candidate.symbol.clone(),
            file_path: candidate.file_path.clone(),
            code,
            start_line,
            end_line,
        });
    }

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
    })
}

/// Reads source lines `[start_line, end_line]` (1-indexed, inclusive) and formats with line numbers.
fn get_source_snippet(
    cache: &mut HashMap<PathBuf, Vec<String>>,
    file_path: &Path,
    start_line: usize,
    end_line: usize,
) -> Result<String, ExploreError> {
    let lines = match cache.get(file_path) {
        Some(lines) => lines,
        None => {
            let file = File::open(file_path).map_err(|err| ExploreError::Io {
                path: file_path.to_path_buf(),
                source: err,
            })?;
            let reader = BufReader::new(file);
            let lines: Result<Vec<String>, std::io::Error> = reader.lines().collect();
            let lines = lines.map_err(|err| ExploreError::Io {
                path: file_path.to_path_buf(),
                source: err,
            })?;
            cache.entry(file_path.to_path_buf()).or_insert(lines)
        }
    };

    if lines.is_empty() {
        return Ok(String::new());
    }

    let actual_start = if start_line == 0 { 1 } else { start_line };
    let actual_end = if end_line < actual_start {
        actual_start
    } else {
        end_line
    };

    let mut snippet = Vec::new();
    for line_idx in actual_start..=actual_end {
        if let Some(line_content) = line_idx.checked_sub(1).and_then(|idx| lines.get(idx)) {
            snippet.push(format!("{line_idx}: {line_content}"));
        }
    }

    Ok(snippet.join("\n"))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/explore.rs"]
mod tests;
