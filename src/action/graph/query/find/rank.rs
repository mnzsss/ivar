use std::collections::HashMap;

use rusqlite::params;

use super::candidate::{ScoredCandidate, add_score};
use super::intent::resolve_query_paths;
use crate::action::graph::query::types::{QueryError, SymbolLocation, map_symbol_and_path_row};
use crate::domain::graph::{FileMention, MentionedSymbol, Symbol};
use crate::store::graph::db::GraphDb;

/// Symbols an explore answer shows source for, and the matching files it leaves out.
#[derive(Debug, Clone, Default)]
pub struct ExploreCandidates {
    pub symbols: Vec<SymbolLocation>,
    pub not_shown: Vec<FileMention>,
}

/// Maximum candidates returned for hero explore.
pub const MAX_EXPLORE_CANDIDATES: usize = 24;
/// Maximum candidate symbols admitted from a single file when multiple files are matched.
pub const MAX_SYMBOLS_PER_FILE: usize = 6;

/// Checks if a file path is a test file.
pub fn is_test_path(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path.split('/')
        .any(|segment| matches!(segment, "test" | "tests" | "__tests__" | "spec" | "e2e"))
        || [".test.", ".spec.", "_test."]
            .iter()
            .any(|marker| path.contains(marker))
}

/// Finds candidate symbols matching an exploration query using structured path pinning,
/// weighted OR search, and per-file candidate limits.
pub fn explore_find_candidates(
    db: &GraphDb,
    query: &str,
    repo: Option<&str>,
) -> Result<Vec<SymbolLocation>, QueryError> {
    Ok(explore_find(db, query, repo, usize::MAX)?.symbols)
}

/// Ranks files for an exploration the way [`explore_find_candidates`] does, keeps
/// candidates from the best `max_files` files, and names the other matching files
/// with their best symbols.
pub fn explore_find(
    db: &GraphDb,
    query: &str,
    repo: Option<&str>,
    max_files: usize,
) -> Result<ExploreCandidates, QueryError> {
    let parsed = resolve_query_paths(db, query, repo)?;
    let conn = db.conn();

    // Map: (repo, file_path) -> Vec<ScoredCandidate>
    let mut file_candidates: HashMap<(String, String), Vec<ScoredCandidate>> = HashMap::new();
    // Track pinned files: (repo, file_path) -> bool
    let mut pinned_files: HashMap<(String, String), bool> = HashMap::new();

    // Step 1: Collect symbols from resolved paths (pinned files and candidate paths).
    for res_path in &parsed.resolved_paths {
        let is_pinned = res_path.is_pinned();
        for (file_id, f_repo, f_path) in res_path.files() {
            let key = (f_repo.clone(), f_path.clone());
            if is_pinned {
                pinned_files.insert(key.clone(), true);
            }

            let mut stmt = conn.prepare_cached(
                "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                        s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path
                 FROM visible_symbols s
                 JOIN visible_files f ON s.file_id = f.id
                 WHERE s.file_id = ?1
                 ORDER BY s.start_line ASC, s.start_col ASC
                 LIMIT 50",
            )?;

            let symbols: Vec<(Symbol, String)> = stmt
                .query_map(params![file_id], map_symbol_and_path_row)?
                .filter_map(|r| r.ok())
                .collect();

            let entry = file_candidates.entry(key).or_default();
            for (symbol, file_path) in symbols {
                if !entry.iter().any(|c| c.symbol.id == symbol.id) {
                    let score = if is_pinned { 100.0 } else { 20.0 };
                    entry.push(ScoredCandidate {
                        symbol,
                        file_path,
                        score,
                    });
                }
            }
        }
    }

    // Step 2: Weighted multi-tier search for remaining terms (Exact > Prefix > Route > FTS5).
    let mut terms_to_search = parsed.search_terms.clone();
    if terms_to_search.is_empty() && parsed.resolved_paths.is_empty() {
        let clean = query
            .trim()
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
        if !clean.is_empty() {
            terms_to_search.push(clean.to_owned());
        }
    }

    for term in &terms_to_search {
        if term.is_empty() {
            continue;
        }

        // Tier A: Exact symbol name match (+100.0)
        {
            let mut stmt = conn.prepare_cached(
                "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                        s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path
                 FROM visible_symbols s
                 JOIN visible_files f ON s.file_id = f.id
                 WHERE s.name = ?1
                   AND (?2 IS NULL OR s.repo = ?2)
                 LIMIT 50",
            )?;
            let rows = stmt
                .query_map(params![term, repo], map_symbol_and_path_row)?
                .filter_map(|r| r.ok());
            for (symbol, file_path) in rows {
                add_score(&mut file_candidates, symbol, file_path, 100.0);
            }
        }

        // Tier B: Prefix symbol name match (+40.0 at a word boundary, +10.0 inside a word)
        if term.len() >= 3 {
            let prefix_pat = format!("{term}%");
            let mut stmt = conn.prepare_cached(
                "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                        s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path
                 FROM visible_symbols s
                 JOIN visible_files f ON s.file_id = f.id
                 WHERE s.name LIKE ?1
                   AND s.name != ?2
                   AND (?3 IS NULL OR s.repo = ?3)
                 LIMIT 50",
            )?;
            let rows = stmt
                .query_map(params![prefix_pat, term, repo], map_symbol_and_path_row)?
                .filter_map(|r| r.ok());
            for (symbol, file_path) in rows {
                let at_word_boundary = match symbol.name.get(term.len()..) {
                    Some(rest) => rest.chars().next().is_none_or(|next| {
                        next.is_ascii_uppercase() || next.is_ascii_digit() || next == '_'
                    }),
                    None => false,
                };
                let score = if at_word_boundary { 40.0 } else { 10.0 };
                add_score(&mut file_candidates, symbol, file_path, score);
            }
        }

        // Path tier: the term, or its plural, names a directory or file in the symbol's path (+60.0)
        {
            let mut stmt = conn.prepare_cached(
                "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                        s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path
                 FROM visible_symbols s
                 JOIN visible_files f ON s.file_id = f.id
                 WHERE (instr('/' || lower(f.path), '/' || lower(?1) || '/') > 0
                        OR instr('/' || lower(f.path), '/' || lower(?1) || '.') > 0
                        OR instr('/' || lower(f.path), '/' || lower(?1) || 's/') > 0
                        OR instr('/' || lower(f.path), '/' || lower(?1) || 's.') > 0)
                   AND (?2 IS NULL OR s.repo = ?2)
                 LIMIT 200",
            )?;
            let rows = stmt
                .query_map(params![term, repo], map_symbol_and_path_row)?
                .filter_map(|r| r.ok());
            for (symbol, file_path) in rows {
                add_score(&mut file_candidates, symbol, file_path, 60.0);
            }
        }

        // Word tier: the term, or its plural, is a whole word inside a camelCase or snake_case name (+30.0)
        let word_query = format!("name_words : (\"{term}\" OR \"{term}s\")");
        if let Ok(mut stmt) = conn.prepare_cached(
            "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                    s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path
             FROM symbols_fts fts
             JOIN symbols s ON fts.rowid = s.id
             JOIN files f ON s.file_id = f.id
             WHERE symbols_fts MATCH ?1
               AND (?2 IS NULL OR s.repo = ?2)
             LIMIT 50",
        ) && let Ok(rows) = stmt.query_map(params![word_query, repo], map_symbol_and_path_row)
        {
            for (symbol, file_path) in rows.filter_map(|r| r.ok()) {
                add_score(&mut file_candidates, symbol, file_path, 30.0);
            }
        }

        // Tier C: FTS5 full-text index (+15.0)
        let fts_query = format!("\"{term}\"*");
        if let Ok(mut stmt) = conn.prepare_cached(
            "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                    s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path
             FROM symbols_fts fts
             JOIN symbols s ON fts.rowid = s.id
             JOIN files f ON s.file_id = f.id
             WHERE symbols_fts MATCH ?1
               AND (?2 IS NULL OR s.repo = ?2)
             ORDER BY fts.rank
             LIMIT 50",
        ) && let Ok(rows) = stmt.query_map(params![fts_query, repo], map_symbol_and_path_row)
        {
            for (symbol, file_path) in rows.filter_map(|r| r.ok()) {
                add_score(&mut file_candidates, symbol, file_path, 15.0);
            }
        }
    }

    // Tier D: HTTP route symbols when the query asks about routes (+30.0)
    if parsed.route_intent {
        let mut stmt = conn.prepare_cached(
            "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                    s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path
             FROM visible_symbols s
             JOIN visible_files f ON s.file_id = f.id
             WHERE lower(s.kind) = 'route'
               AND (?1 IS NULL OR s.repo = ?1)
             ORDER BY f.path ASC, s.start_line ASC
             LIMIT 50",
        )?;
        let rows = stmt
            .query_map(params![repo], map_symbol_and_path_row)?
            .filter_map(|r| r.ok());
        for (symbol, file_path) in rows {
            add_score(&mut file_candidates, symbol, file_path, 30.0);
        }
    }

    // Boost route intent if detected (+30.0 for route-like symbols)
    if parsed.route_intent {
        for ((_, file_path), candidates) in file_candidates.iter_mut() {
            let is_route_file = file_path.contains("route")
                || file_path.contains("api")
                || file_path.contains("endpoint")
                || file_path.contains("handler")
                || file_path.contains("controller");
            for cand in candidates.iter_mut() {
                let is_route_symbol = is_route_file
                    || cand.symbol.name.to_ascii_lowercase().contains("route")
                    || cand.symbol.name.to_ascii_lowercase().contains("handler")
                    || cand.symbol.name.to_ascii_lowercase().contains("get")
                    || cand.symbol.name.to_ascii_lowercase().contains("post")
                    || cand.symbol.name.to_ascii_lowercase().contains("api");
                if is_route_symbol {
                    cand.score += 30.0;
                }
            }
        }
    }

    if file_candidates.is_empty() {
        return Ok(ExploreCandidates::default());
    }

    // Step 3: Compute aggregate score per file and rank files.
    struct FileScore {
        repo: String,
        path: String,
        score: f64,
    }

    let asks_for_tests = parsed.search_terms.iter().any(|term| {
        let term = term.to_ascii_lowercase();
        term.starts_with("test") || term.starts_with("spec")
    });
    let mut ranked_files: Vec<FileScore> = file_candidates
        .iter()
        .map(|((r, p), cands)| {
            let is_pinned = pinned_files
                .get(&(r.clone(), p.clone()))
                .copied()
                .unwrap_or(false);
            let mut sum_score = 0.0;
            for c in cands {
                sum_score += c.score;
            }
            if !asks_for_tests && is_test_path(p) {
                sum_score *= 0.5;
            }
            let base = if is_pinned { 10000.0 } else { 0.0 };
            FileScore {
                repo: r.clone(),
                path: p.clone(),
                score: base + sum_score,
            }
        })
        .collect();

    ranked_files.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.repo.cmp(&b.repo))
            .then_with(|| a.path.cmp(&b.path))
    });

    let floor = ranked_files.first().map_or(0.0, |top| top.score * 0.25);
    ranked_files.retain(|file| file.score >= floor);

    let shown_files = ranked_files.len().min(max_files);
    let max_per_file = match shown_files {
        1 => MAX_EXPLORE_CANDIDATES,
        files => (MAX_EXPLORE_CANDIDATES / files).clamp(1, MAX_SYMBOLS_PER_FILE),
    };

    // Step 4: Collect symbols respecting per-file caps and preserve source line order.
    let mut final_candidates = Vec::new();
    let mut not_shown = Vec::new();

    for (rank, file_info) in ranked_files.into_iter().enumerate() {
        let key = (file_info.repo, file_info.path);
        let Some(mut cands) = file_candidates.remove(&key) else {
            continue;
        };
        cands.sort_by(|a, b| b.score.total_cmp(&a.score));
        if rank < shown_files && final_candidates.len() < MAX_EXPLORE_CANDIDATES {
            cands.truncate((MAX_EXPLORE_CANDIDATES - final_candidates.len()).min(max_per_file));
            cands.sort_by_key(|c| (c.symbol.span.start_line, c.symbol.span.start_col));
            final_candidates.extend(cands.into_iter().map(|sc| SymbolLocation {
                symbol: sc.symbol,
                file_path: sc.file_path,
            }));
        } else {
            cands.truncate(MAX_SYMBOLS_PER_FILE);
            cands.sort_by_key(|c| (c.symbol.span.start_line, c.symbol.span.start_col));
            let (repo, file_path) = key;
            not_shown.push(FileMention {
                repo,
                file_path,
                symbols: cands
                    .into_iter()
                    .map(|sc| MentionedSymbol {
                        line: sc.symbol.span.start_line,
                        name: sc.symbol.name,
                    })
                    .collect(),
            });
        }
    }

    Ok(ExploreCandidates {
        symbols: final_candidates,
        not_shown,
    })
}
