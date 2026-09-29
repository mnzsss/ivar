use std::collections::HashMap;

use rusqlite::params;

use super::{FileCandidates, PATH_TIER_SQL, PinnedFiles};
use crate::action::graph::query::find::candidate::{ScoredCandidate, add_score};
use crate::action::graph::query::find::intent::{ParsedExploreQuery, ResolvedPath};
use crate::action::graph::query::find::search::{NAME_PREFIX_MATCH, prefix_casings};
use crate::action::graph::query::types::{QueryError, map_symbol_and_path_row};
use crate::domain::graph::Symbol;

/// Step 1: Collect symbols from resolved paths (pinned files and candidate paths).
pub(super) fn collect_pinned_candidates(
    conn: &rusqlite::Connection,
    parsed: &ParsedExploreQuery,
) -> Result<(FileCandidates, PinnedFiles), QueryError> {
    let mut file_candidates: FileCandidates = HashMap::new();
    let mut pinned_files: PinnedFiles = HashMap::new();

    for res_path in &parsed.resolved_paths {
        let is_pinned = res_path.is_pinned();
        let is_dir = matches!(res_path, ResolvedPath::DirectorySubtree { .. });
        if !is_pinned && !is_dir {
            continue;
        }
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
                .collect::<Result<_, _>>()?;

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

    Ok((file_candidates, pinned_files))
}

pub(super) fn search_terms_for(query: &str, parsed: &ParsedExploreQuery) -> Vec<String> {
    let mut terms_to_search = parsed.search_terms.clone();
    if terms_to_search.is_empty() && parsed.resolved_paths.is_empty() {
        let clean = query
            .trim()
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
        if !clean.is_empty() {
            terms_to_search.push(clean.to_owned());
        }
    }
    terms_to_search
}

/// Step 2: Weighted multi-tier search for remaining terms (Exact > Prefix > Route > FTS5).
pub(super) fn score_matching_terms(
    conn: &rusqlite::Connection,
    terms: &[String],
    repo: Option<&str>,
    route_intent: bool,
    file_candidates: &mut FileCandidates,
) -> Result<(), QueryError> {
    for term in terms {
        if term.is_empty() {
            continue;
        }
        score_term_tiers(conn, term, repo, file_candidates)?;
    }

    // Tier D: HTTP route symbols when the query asks about routes (+30.0)
    if route_intent {
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
            .collect::<Result<Vec<_>, _>>()?;
        for (symbol, file_path) in rows {
            add_score(file_candidates, symbol, file_path, 30.0);
        }
    }

    Ok(())
}

/// Scores a single search term across the exact, prefix, path, word, and FTS5 tiers.
fn score_term_tiers(
    conn: &rusqlite::Connection,
    term: &str,
    repo: Option<&str>,
    file_candidates: &mut FileCandidates,
) -> Result<(), QueryError> {
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
            .collect::<Result<Vec<_>, _>>()?;
        for (symbol, file_path) in rows {
            add_score(file_candidates, symbol, file_path, 100.0);
        }
    }

    // Tier B: Prefix symbol name match (+40.0 at a word boundary, +10.0 inside a word)
    if term.len() >= 3 {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                    s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path
             FROM visible_symbols s
             JOIN visible_files f ON s.file_id = f.id
             WHERE {NAME_PREFIX_MATCH}
               AND s.name != ?5
               AND (?6 IS NULL OR s.repo = ?6)
             ORDER BY s.id ASC
             LIMIT 50",
        ))?;
        let [c1, c2, c3, c4] = prefix_casings(term);
        let rows = stmt
            .query_map(params![c1, c2, c3, c4, term, repo], map_symbol_and_path_row)?
            .collect::<Result<Vec<_>, _>>()?;
        for (symbol, file_path) in rows {
            let at_word_boundary = match symbol.name.get(term.len()..) {
                Some(rest) => rest.chars().next().is_none_or(|next| {
                    next.is_ascii_uppercase() || next.is_ascii_digit() || next == '_'
                }),
                None => false,
            };
            let score = if at_word_boundary { 40.0 } else { 10.0 };
            add_score(file_candidates, symbol, file_path, score);
        }
    }

    // Path tier: the term, or its plural, names a directory or file in the symbol's path (+60.0)
    {
        let mut stmt = conn.prepare_cached(PATH_TIER_SQL)?;
        let rows = stmt
            .query_map(params![term, repo], map_symbol_and_path_row)?
            .collect::<Result<Vec<_>, _>>()?;
        for (symbol, file_path) in rows {
            add_score(file_candidates, symbol, file_path, 60.0);
        }
    }

    // Word tier: the term, or its plural, is a whole word inside a camelCase or snake_case name (+30.0)
    let word_query = format!("name_words : (\"{term}\" OR \"{term}s\")");
    if let Ok(mut stmt) = conn.prepare_cached(
        "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path
         FROM symbols_fts fts
         JOIN visible_symbols s ON fts.rowid = s.id
         JOIN visible_files f ON s.file_id = f.id
         WHERE symbols_fts MATCH ?1
           AND (?2 IS NULL OR s.repo = ?2)
         LIMIT 50",
    ) {
        let rows = stmt.query_map(params![word_query, repo], map_symbol_and_path_row)?;
        for row in rows {
            let (symbol, file_path) = row?;
            add_score(file_candidates, symbol, file_path, 30.0);
        }
    }

    // Tier C: FTS5 full-text index (+15.0)
    let fts_query = format!("\"{term}\"*");
    if let Ok(mut stmt) = conn.prepare_cached(
        "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path
         FROM symbols_fts fts
         JOIN visible_symbols s ON fts.rowid = s.id
         JOIN visible_files f ON s.file_id = f.id
         WHERE symbols_fts MATCH ?1
           AND (?2 IS NULL OR s.repo = ?2)
         ORDER BY fts.rank
         LIMIT 50",
    ) {
        let rows = stmt.query_map(params![fts_query, repo], map_symbol_and_path_row)?;
        for row in rows {
            let (symbol, file_path) = row?;
            add_score(file_candidates, symbol, file_path, 15.0);
        }
    }

    Ok(())
}

/// Boost route intent if detected (+30.0 for route-like symbols)
pub(super) fn boost_route_intent_symbols(file_candidates: &mut FileCandidates) {
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
