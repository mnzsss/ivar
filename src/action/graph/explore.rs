//! Hero explore query engine synthesizing symbol discovery, surgical source code spans,
//! immediate call flows, and blast-radius impact analysis.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::action::graph::query::{self, QueryError};
use crate::domain::graph::{CallFlowItem, ExploreResult, SymbolSnippet};
use crate::infra::graph::db::GraphDb;

/// Errors that can occur during explore synthesis.
#[derive(Debug, thiserror::Error)]
pub enum ExploreError {
    #[error("Database query failed: {0}")]
    Query(#[from] QueryError),
    #[error("Database error: {0}")]
    Db(#[from] crate::infra::graph::db::GraphDbError),
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
            query: query.to_string(),
            primary_symbols: Vec::new(),
            call_flows: Vec::new(),
            impact_summary: None,
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
                    if !candidates.iter().any(|existing| existing.symbol.id == c.symbol.id) {
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
            query: query.to_string(),
            primary_symbols: Vec::new(),
            call_flows: Vec::new(),
            impact_summary: Some(format!("No symbols found matching '{query}'.")),
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

    // Step 3: Immediate call flows (for primary symbols)
    let mut call_flows = Vec::new();
    let mut seen_flows = std::collections::HashSet::new();

    for candidate in &candidates {
        // Query callers
        let callers = query::get_callers(db, &candidate.symbol.name, repo, true, 0.7)?;
        for caller in callers {
            let flow_key = (
                caller.caller.name.clone(),
                candidate.symbol.name.clone(),
                caller.line,
            );
            if seen_flows.insert(flow_key) {
                call_flows.push(CallFlowItem {
                    caller: caller.caller.name,
                    callee: candidate.symbol.name.clone(),
                    edge_kind: caller.edge_kind,
                    provenance: caller.provenance,
                    line: caller.line,
                });
            }
        }

        // Query callees
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
                        callee: callee.callee_name,
                        edge_kind: callee.edge_kind,
                        provenance: callee.provenance,
                        line: callee.line,
                    });
                }
            }
        }
    }

    // Step 4: Blast radius / impact summary
    let impact_summary = if let Some(primary) = candidates.first() {
        if let Some(sym_id) = primary.symbol.id {
            let impact = query::get_impact(db, sym_id, 3)?;
            let total_callers = impact.total_affected;
            let total_files = impact.affected_files.len();
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
        query: query.to_string(),
        primary_symbols,
        call_flows,
        impact_summary,
    })
}

/// Reads source lines `[start_line, end_line]` (1-indexed, inclusive) and formats with line numbers.
fn get_source_snippet(
    cache: &mut HashMap<PathBuf, Vec<String>>,
    file_path: &Path,
    start_line: usize,
    end_line: usize,
) -> Result<String, ExploreError> {
    if !cache.contains_key(file_path) {
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
        cache.insert(file_path.to_path_buf(), lines);
    }

    let lines = cache.get(file_path).unwrap();
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
        if line_idx <= lines.len() {
            let line_content = &lines[line_idx - 1];
            snippet.push(format!("{line_idx}: {line_content}"));
        }
    }

    Ok(snippet.join("\n"))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/explore.rs"]
mod tests;
