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
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    use crate::domain::graph::{EdgeKind, Provenance, Span, Symbol, SymbolKind};

    #[test]
    fn test_explore_hero_query() {
        let temp = tempdir().expect("create temp dir");
        let hall_root = temp.path();

        // Create mock repository directory and source file
        let repo_dir = hall_root.join("test-repo");
        fs::create_dir_all(repo_dir.join("src")).expect("create src dir");
        let file_rel_path = "src/hall.rs";
        let file_content = r#"// Header comment
pub fn init_hall(config: Config) -> Result<Hall> {
    let hall = Hall::new(config);
    setup_logging(&hall);
    Ok(hall)
}

pub fn caller_func() {
    init_hall(Config::default());
}
"#;
        fs::write(repo_dir.join(file_rel_path), file_content).expect("write source file");

        let db = GraphDb::open_in_memory().expect("open memory db");
        db.insert_repo(
            "test-repo",
            repo_dir.to_str().unwrap(),
            "main",
            Some("commit1"),
        )
        .expect("insert repo");

        let file_id = db
            .upsert_file("test-repo", file_rel_path, "hash1", 1000, file_content.len() as i64)
            .expect("upsert file");

        let init_hall_sym = Symbol {
            id: None,
            file_id: Some(file_id),
            repo: "test-repo".to_string(),
            name: "init_hall".to_string(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("pub fn init_hall(config: Config) -> Result<Hall>".to_string()),
            docstring: None,
            span: Span::new(2, 1, 6, 2),
            is_exported: true,
        };

        let caller_sym = Symbol {
            id: None,
            file_id: Some(file_id),
            repo: "test-repo".to_string(),
            name: "caller_func".to_string(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("pub fn caller_func()".to_string()),
            docstring: None,
            span: Span::new(8, 1, 10, 2),
            is_exported: true,
        };

        let sym_ids = db
            .insert_symbols(&[init_hall_sym, caller_sym])
            .expect("insert symbols");
        let init_hall_id = sym_ids[0];
        let caller_func_id = sym_ids[1];

        // Add edge: caller_func -> init_hall (CALLS)
        let edge1 = crate::domain::graph::Edge {
            id: None,
            file_id: Some(file_id),
            repo: "test-repo".to_string(),
            from_symbol_id: Some(caller_func_id),
            to_symbol_id: Some(init_hall_id),
            to_name: Some("init_hall".to_string()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 9,
            col: 5,
            confidence: 1.0,
        };

        // Add edge: init_hall -> setup_logging (CALLS, unresolved)
        let edge2 = crate::domain::graph::Edge {
            id: None,
            file_id: Some(file_id),
            repo: "test-repo".to_string(),
            from_symbol_id: Some(init_hall_id),
            to_symbol_id: None,
            to_name: Some("setup_logging".to_string()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 4,
            col: 5,
            confidence: 1.0,
        };

        db.insert_edges(&[edge1, edge2]).expect("insert edges");

        // Run explore
        let result = explore(&db, hall_root, "init_hall", Some("test-repo"))
            .expect("explore should succeed");

        assert_eq!(result.query, "init_hall");
        assert_eq!(result.primary_symbols.len(), 1);

        let snippet = &result.primary_symbols[0];
        assert_eq!(snippet.symbol.name, "init_hall");
        assert_eq!(snippet.file_path, "src/hall.rs");
        assert_eq!(snippet.start_line, 2);
        assert_eq!(snippet.end_line, 6);

        // Verbatim code lines check
        assert!(snippet.code.contains("2: pub fn init_hall(config: Config) -> Result<Hall> {"));
        assert!(snippet.code.contains("4:     setup_logging(&hall);"));
        assert!(snippet.code.contains("6: }"));

        // Call flows check
        assert_eq!(result.call_flows.len(), 2);
        let caller_flow = result.call_flows.iter().find(|f| f.caller == "caller_func").unwrap();
        assert_eq!(caller_flow.callee, "init_hall");
        assert_eq!(caller_flow.line, 9);

        let callee_flow = result.call_flows.iter().find(|f| f.callee == "setup_logging").unwrap();
        assert_eq!(callee_flow.caller, "init_hall");
        assert_eq!(callee_flow.line, 4);

        // Impact summary check
        assert!(result.impact_summary.is_some());
        let summary = result.impact_summary.unwrap();
        assert!(summary.contains("Modifying 'init_hall' directly impacts 1 caller across 1 file."));
    }

    #[test]
    fn test_explore_empty_or_not_found() {
        let temp = tempdir().expect("create temp dir");
        let db = GraphDb::open_in_memory().expect("open memory db");

        let empty_res = explore(&db, temp.path(), "   ", None).expect("empty query");
        assert!(empty_res.primary_symbols.is_empty());
        assert!(empty_res.call_flows.is_empty());

        let not_found_res = explore(&db, temp.path(), "non_existent_func", None).expect("not found");
        assert!(not_found_res.primary_symbols.is_empty());
        assert!(not_found_res.impact_summary.unwrap().contains("No symbols found"));
    }
}
