//! Symbol search: exact, prefix, and FTS5 lookup for `find_symbols`, plus path
//! pinning, weighted OR ranking, and per-file candidate limiting for explore.

use std::collections::HashMap;

use rusqlite::params;

use super::types::{QueryError, SymbolLocation, map_symbol_and_path_row};
use crate::domain::graph::Symbol;
use crate::store::graph::db::GraphDb;

/// Maximum candidates returned for hero explore.
pub const MAX_EXPLORE_CANDIDATES: usize = 24;
/// Maximum candidate symbols admitted from a single file when multiple files are matched.
pub const MAX_SYMBOLS_PER_FILE: usize = 6;

/// Candidate symbol associated with its file path and search score.
#[derive(Debug, Clone)]
pub struct ScoredCandidate {
    pub symbol: Symbol,
    pub file_path: String,
    pub score: f64,
}

/// Resolved path target from query parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedPath {
    /// Exact file match (pinned).
    ExactFile {
        file_id: i64,
        repo: String,
        path: String,
    },
    /// Workspace-relative file match (pinned).
    WorkspaceRelative {
        file_id: i64,
        repo: String,
        path: String,
    },
    /// Single unambiguous basename match across the database (pinned).
    UnambiguousBasename {
        file_id: i64,
        repo: String,
        path: String,
    },
    /// Multiple files matched a basename (unpinned candidates).
    AmbiguousBasename { files: Vec<(i64, String, String)> },
    /// Directory or subtree match (unpinned candidate files).
    DirectorySubtree { files: Vec<(i64, String, String)> },
}

impl ResolvedPath {
    pub fn is_pinned(&self) -> bool {
        matches!(
            self,
            Self::ExactFile { .. }
                | Self::WorkspaceRelative { .. }
                | Self::UnambiguousBasename { .. }
        )
    }

    pub fn files(&self) -> Vec<(i64, String, String)> {
        match self {
            Self::ExactFile {
                file_id,
                repo,
                path,
            }
            | Self::WorkspaceRelative {
                file_id,
                repo,
                path,
            }
            | Self::UnambiguousBasename {
                file_id,
                repo,
                path,
            } => {
                vec![(*file_id, repo.clone(), path.clone())]
            }
            Self::AmbiguousBasename { files } | Self::DirectorySubtree { files } => files.clone(),
        }
    }
}

/// Parsed components of an explore query.
#[derive(Debug, Clone, Default)]
pub struct ParsedExploreQuery {
    pub raw_query: String,
    pub path_tokens: Vec<String>,
    pub resolved_paths: Vec<ResolvedPath>,
    pub search_terms: Vec<String>,
    pub route_intent: bool,
}

/// Checks whether a token looks like a file path or directory.
pub fn is_path_like(token: &str) -> bool {
    let t = token.trim();
    if t.is_empty() {
        return false;
    }
    if t.contains('/') || t.contains('\\') {
        return true;
    }
    matches!(
        t.rsplit_once('.').map(|(_, ext)| ext),
        Some(
            "rs" | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "mjs"
                | "cjs"
                | "json"
                | "md"
                | "toml"
                | "yaml"
                | "yml"
                | "py"
                | "go"
                | "c"
                | "cpp"
                | "h"
                | "hpp"
                | "java"
                | "kt"
                | "swift"
                | "proto"
                | "sql"
                | "graphql"
                | "sh"
        )
    )
}

/// Detects if terms express route/API intent.
///
/// Route intent pulls every route symbol into the candidates, so verbs like
/// `get` or `delete` stay out: they appear in ordinary symbol queries.
fn is_route_intent_term(term: &str) -> bool {
    let lower = term.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "route" | "routes" | "endpoint" | "endpoints" | "api"
    )
}

/// Resolves path tokens against the database to identify exact files, workspace-relative
/// paths, unambiguous basenames, ambiguous basenames, or directory subtrees.
pub fn resolve_query_paths(
    db: &GraphDb,
    query: &str,
    repo: Option<&str>,
) -> Result<ParsedExploreQuery, QueryError> {
    let conn = db.conn();
    let tokens: Vec<&str> = query.split_whitespace().collect();

    let mut path_tokens = Vec::new();
    let mut resolved_paths = Vec::new();
    let mut remaining_words = Vec::new();
    let mut route_intent = false;

    for token in tokens {
        let clean_token =
            token.trim_matches(|c: char| c == ',' || c == ';' || c == ':' || c == '"' || c == '\'');
        if clean_token.is_empty() {
            continue;
        }

        if is_route_intent_term(clean_token) {
            route_intent = true;
        }

        if is_path_like(clean_token) {
            path_tokens.push(clean_token.to_owned());
            let trimmed = clean_token.trim_start_matches("./");

            // 1. Check exact match: f.path = ?
            let mut exact_stmt = conn.prepare_cached(
                "SELECT id, repo, path FROM files WHERE (?1 IS NULL OR repo = ?1) AND path = ?2 LIMIT 2",
            )?;
            let exact_matches: Vec<(i64, String, String)> = exact_stmt
                .query_map(params![repo, trimmed], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?
                .filter_map(|r| r.ok())
                .collect();

            if let [(file_id, r, p)] = exact_matches.as_slice() {
                resolved_paths.push(ResolvedPath::ExactFile {
                    file_id: *file_id,
                    repo: r.clone(),
                    path: p.clone(),
                });
                continue;
            }

            // 2. Check workspace-relative / suffix match
            let mut rel_stmt = conn.prepare_cached(
                "SELECT id, repo, path FROM files
                 WHERE (?1 IS NULL OR repo = ?1)
                   AND (
                     path = ?2
                     OR path LIKE '%/' || ?2 ESCAPE '\\'
                     OR ?2 LIKE '%/' || path ESCAPE '\\'
                   )
                 LIMIT 10",
            )?;
            let rel_matches: Vec<(i64, String, String)> = rel_stmt
                .query_map(params![repo, trimmed], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?
                .filter_map(|r| r.ok())
                .collect();

            if let [(file_id, r, p)] = rel_matches.as_slice() {
                resolved_paths.push(ResolvedPath::WorkspaceRelative {
                    file_id: *file_id,
                    repo: r.clone(),
                    path: p.clone(),
                });
                continue;
            } else if rel_matches.len() > 1 && !trimmed.contains('/') && !trimmed.contains('\\') {
                resolved_paths.push(ResolvedPath::AmbiguousBasename { files: rel_matches });
                continue;
            }

            // 3. Basename or Directory/Subtree match
            let clean_dir = trimmed.trim_end_matches('/');
            let mut dir_stmt = conn.prepare_cached(
                "SELECT id, repo, path FROM files
                 WHERE (?1 IS NULL OR repo = ?1)
                   AND (
                     path LIKE ?2 || '/%' ESCAPE '\\'
                     OR path LIKE '%/' || ?2 || '/%' ESCAPE '\\'
                   )
                 LIMIT 50",
            )?;
            let dir_matches: Vec<(i64, String, String)> = dir_stmt
                .query_map(params![repo, clean_dir], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?
                .filter_map(|r| r.ok())
                .collect();

            if !dir_matches.is_empty() {
                resolved_paths.push(ResolvedPath::DirectorySubtree { files: dir_matches });
                continue;
            }

            // 4. Basename lookup across database
            let mut base_stmt = conn.prepare_cached(
                "SELECT id, repo, path FROM files
                 WHERE (?1 IS NULL OR repo = ?1)
                   AND (path = ?2 OR path LIKE '%/' || ?2 ESCAPE '\\')
                 LIMIT 10",
            )?;
            let base_matches: Vec<(i64, String, String)> = base_stmt
                .query_map(params![repo, trimmed], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?
                .filter_map(|r| r.ok())
                .collect();

            if let [(file_id, r, p)] = base_matches.as_slice() {
                resolved_paths.push(ResolvedPath::UnambiguousBasename {
                    file_id: *file_id,
                    repo: r.clone(),
                    path: p.clone(),
                });
            } else if base_matches.len() > 1 {
                resolved_paths.push(ResolvedPath::AmbiguousBasename {
                    files: base_matches,
                });
            }
        } else {
            let clean_term = clean_token.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if !clean_term.is_empty() {
                remaining_words.push(clean_term.to_owned());
            }
        }
    }

    Ok(ParsedExploreQuery {
        raw_query: query.to_owned(),
        path_tokens,
        resolved_paths,
        search_terms: remaining_words,
        route_intent,
    })
}

/// Adds `score` to a symbol's candidate entry, creating the entry on first sight.
fn add_score(
    file_candidates: &mut HashMap<(String, String), Vec<ScoredCandidate>>,
    symbol: Symbol,
    file_path: String,
    score: f64,
) {
    let entry = file_candidates
        .entry((symbol.repo.clone(), file_path.clone()))
        .or_default();
    if let Some(existing) = entry.iter_mut().find(|c| c.symbol.id == symbol.id) {
        existing.score += score;
    } else {
        entry.push(ScoredCandidate {
            symbol,
            file_path,
            score,
        });
    }
}

/// Finds candidate symbols matching an exploration query using structured path pinning,
/// weighted OR search, and per-file candidate limits.
pub fn explore_find_candidates(
    db: &GraphDb,
    query: &str,
    repo: Option<&str>,
) -> Result<Vec<SymbolLocation>, QueryError> {
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
                 FROM symbols s
                 JOIN files f ON s.file_id = f.id
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
    // If no remaining terms and no paths, use the full query as a term
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
                 FROM symbols s
                 JOIN files f ON s.file_id = f.id
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
                 FROM symbols s
                 JOIN files f ON s.file_id = f.id
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
                 FROM symbols s
                 JOIN files f ON s.file_id = f.id
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
             FROM symbols s
             JOIN files f ON s.file_id = f.id
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
        return Ok(Vec::new());
    }

    // Step 3: Compute aggregate score per file and rank files.
    struct FileScore {
        repo: String,
        path: String,
        score: f64,
    }

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
            // Bonus for pinned files
            let base = if is_pinned { 10000.0 } else { 0.0 };
            FileScore {
                repo: r.clone(),
                path: p.clone(),
                score: base + sum_score,
            }
        })
        .collect();

    // Sort files: highest score first. HashMap iteration order is random, so ties
    // fall back to (repo, path) to keep output identical across runs.
    ranked_files.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.repo.cmp(&b.repo))
            .then_with(|| a.path.cmp(&b.path))
    });

    // Files far below the best match were decoys sharing a name prefix, and each
    // one costs a whole file of source in the answer.
    let floor = ranked_files.first().map_or(0.0, |top| top.score * 0.25);
    ranked_files.retain(|file| file.score >= floor);

    // Split the budget across files so every file the query names keeps a slot.
    let max_per_file = match ranked_files.len() {
        1 => MAX_EXPLORE_CANDIDATES,
        files => (MAX_EXPLORE_CANDIDATES / files).clamp(1, MAX_SYMBOLS_PER_FILE),
    };

    // Step 4: Collect symbols respecting per-file caps and preserve source line order.
    let mut final_candidates = Vec::new();

    for file_info in ranked_files {
        if final_candidates.len() >= MAX_EXPLORE_CANDIDATES {
            break;
        }

        let key = (file_info.repo, file_info.path);
        if let Some(mut cands) = file_candidates.remove(&key) {
            // Cap by score before restoring source order, or a strong match late in
            // the file never survives the cap.
            cands.sort_by(|a, b| b.score.total_cmp(&a.score));
            cands.truncate((MAX_EXPLORE_CANDIDATES - final_candidates.len()).min(max_per_file));
            cands.sort_by_key(|c| (c.symbol.span.start_line, c.symbol.span.start_col));
            final_candidates.extend(cands.into_iter().map(|sc| SymbolLocation {
                symbol: sc.symbol,
                file_path: sc.file_path,
            }));
        }
    }

    Ok(final_candidates)
}

/// Finds symbols by exact name, prefix, and full-text search.
pub fn find_symbols(
    db: &GraphDb,
    query: &str,
    repo: Option<&str>,
    limit: usize,
) -> Result<Vec<SymbolLocation>, QueryError> {
    if limit == 0 || query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let conn = db.conn();
    let mut results = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    // 1. Exact name matches
    {
        let mut stmt = conn.prepare_cached(
            "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                    s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path
             FROM symbols s
             JOIN files f ON s.file_id = f.id
             WHERE s.name = ?1 AND (?2 IS NULL OR s.repo = ?2)
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![query, repo, limit as i64], map_symbol_and_path_row)?;
        for row in rows {
            let (sym, path) = row?;
            if let Some(id) = sym.id
                && seen_ids.insert(id)
            {
                results.push(SymbolLocation {
                    symbol: sym,
                    file_path: path,
                });
                if results.len() >= limit {
                    return Ok(results);
                }
            }
        }
    }

    // 2. Prefix matches
    if results.len() < limit {
        let prefix_query = format!("{query}%");
        let mut stmt = conn.prepare_cached(
            "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                    s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path
             FROM symbols s
             JOIN files f ON s.file_id = f.id
             WHERE s.name LIKE ?1 AND (?2 IS NULL OR s.repo = ?2)
             ORDER BY length(s.name) ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![prefix_query, repo, limit as i64],
            map_symbol_and_path_row,
        )?;
        for row in rows {
            let (sym, path) = row?;
            if let Some(id) = sym.id
                && seen_ids.insert(id)
            {
                results.push(SymbolLocation {
                    symbol: sym,
                    file_path: path,
                });
                if results.len() >= limit {
                    return Ok(results);
                }
            }
        }
    }

    // 3. FTS5 search
    if results.len() < limit {
        let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
        if let Ok(mut stmt) = conn.prepare_cached(
            "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                    s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, f.path
             FROM symbols_fts fts
             JOIN symbols s ON fts.rowid = s.id
             JOIN files f ON s.file_id = f.id
             WHERE symbols_fts MATCH ?1 AND (?2 IS NULL OR s.repo = ?2)
             ORDER BY rank
             LIMIT ?3",
        ) && let Ok(rows) = stmt.query_map(
            params![fts_query, repo, limit as i64],
            map_symbol_and_path_row,
        ) {
            for row in rows.flatten() {
                let (sym, path) = row;
                if let Some(id) = sym.id
                    && seen_ids.insert(id)
                {
                    results.push(SymbolLocation {
                        symbol: sym,
                        file_path: path,
                    });
                    if results.len() >= limit {
                        return Ok(results);
                    }
                }
            }
        }
    }

    Ok(results)
}
