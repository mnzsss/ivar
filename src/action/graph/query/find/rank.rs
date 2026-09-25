use std::collections::HashMap;

use rusqlite::params;

use super::candidate::{ScoredCandidate, add_score};
use super::intent::{ParsedExploreQuery, ResolvedPath, resolve_query_paths};
use super::search::{NAME_PREFIX_MATCH, prefix_casings};
use crate::action::graph::query::types::{QueryError, SymbolLocation, map_symbol_and_path_row};
use crate::domain::graph::{FileMatch, FileMatchKind, FileMention, MentionedSymbol, Symbol};
use crate::store::graph::db::GraphDb;

type FileCandidates = HashMap<(String, String), Vec<ScoredCandidate>>;
type PinnedFiles = HashMap<(String, String), bool>;

/// Symbols and files an explore answer shows source for, and the matching files it leaves out.
#[derive(Debug, Clone, Default)]
pub struct ExploreCandidates {
    pub symbols: Vec<SymbolLocation>,
    pub files: Vec<FileMatch>,
    pub not_shown: Vec<FileMention>,
}

impl ExploreCandidates {
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty() && self.files.is_empty()
    }
}

/// Symbols in files whose path names the term `?1` or its plural, repo `?2`.
pub(super) const PATH_TIER_SQL: &str = "WITH matched_files AS MATERIALIZED (
         SELECT id, path FROM visible_files
         WHERE (instr('/' || lower(path), '/' || lower(?1) || '/') > 0
                OR instr('/' || lower(path), '/' || lower(?1) || '.') > 0
                OR instr('/' || lower(path), '/' || lower(?1) || 's/') > 0
                OR instr('/' || lower(path), '/' || lower(?1) || 's.') > 0)
           AND (?2 IS NULL OR repo = ?2)
     ),
     matched_symbols AS MATERIALIZED (
         SELECT * FROM visible_symbols
         WHERE file_id IN (SELECT id FROM matched_files)
           AND (?2 IS NULL OR repo = ?2)
     )
     SELECT m.id, m.file_id, m.repo, m.name, m.kind, m.scope, m.signature, m.docstring,
            m.start_line, m.start_col, m.end_line, m.end_col, m.is_exported, m.complexity, f.path
     FROM matched_symbols m
     JOIN matched_files f ON f.id = m.file_id
     ORDER BY m.id ASC
     LIMIT 200";

/// Maximum candidates returned for hero explore.
pub const MAX_EXPLORE_CANDIDATES: usize = 24;
/// Maximum candidate symbols admitted from a single file when multiple files are matched.
pub const MAX_SYMBOLS_PER_FILE: usize = 6;
/// Maximum file matches returned for exploration.
pub const MAX_FILE_CANDIDATES: usize = 6;

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

#[derive(Debug, Clone)]
struct ContentHitInfo {
    repo: String,
    path: String,
    rank: f64,
    start_line: usize,
    excerpt: String,
    content_truncated: bool,
}

/// Ranks files and symbols for an exploration, keeping
/// candidates from the best `max_files` files, and naming the other matching files
/// with their best symbols.
pub fn explore_find(
    db: &GraphDb,
    query: &str,
    repo: Option<&str>,
    max_files: usize,
) -> Result<ExploreCandidates, QueryError> {
    let parsed = resolve_query_paths(db, query, repo)?;
    let effective_repo = parsed.target_repo.as_deref().or(repo);
    let conn = db.conn();

    let (mut file_candidates, pinned_files) = collect_pinned_candidates(conn, &parsed)?;

    let terms_to_search = search_terms_for(query, &parsed);
    score_matching_terms(
        conn,
        &terms_to_search,
        effective_repo,
        parsed.route_intent,
        &mut file_candidates,
    )?;

    if parsed.route_intent {
        boost_route_intent_symbols(&mut file_candidates);
    }

    // Also collect file content matches
    let content_hits = search_content_hits(db, &terms_to_search, effective_repo)?;

    let ranked_files = rank_all_files(
        &file_candidates,
        &pinned_files,
        &content_hits,
        &parsed,
        effective_repo,
    );

    if ranked_files.is_empty() {
        return Ok(ExploreCandidates::default());
    }

    let shown_files = ranked_files.len().min(max_files);
    let max_per_file = match shown_files {
        1 => MAX_EXPLORE_CANDIDATES,
        files => (MAX_EXPLORE_CANDIDATES / files).clamp(1, MAX_SYMBOLS_PER_FILE),
    };

    Ok(collect_final_results(
        file_candidates,
        &content_hits,
        ranked_files,
        shown_files,
        max_per_file,
    ))
}

fn search_content_hits(
    db: &GraphDb,
    terms: &[String],
    repo: Option<&str>,
) -> Result<HashMap<(String, String), ContentHitInfo>, QueryError> {
    let mut hits = HashMap::new();
    for term in terms {
        if term.trim().is_empty() {
            continue;
        }
        let term_hits = db.search_file_content(term, repo, 50)?;
        for hit in term_hits {
            let key = (hit.repo.clone(), hit.path.clone());
            let (start_line, excerpt) = find_line_and_excerpt(&hit.indexed_content, term);
            hits.entry(key).or_insert(ContentHitInfo {
                repo: hit.repo,
                path: hit.path,
                rank: hit.rank,
                start_line,
                excerpt,
                content_truncated: hit.content_truncated,
            });
        }
    }
    Ok(hits)
}

fn find_line_and_excerpt(content: &str, term: &str) -> (usize, String) {
    let term_lower = term.to_ascii_lowercase();
    for (idx, line) in content.lines().enumerate() {
        if line.to_ascii_lowercase().contains(&term_lower) {
            return (idx + 1, line.trim().to_owned());
        }
    }
    let first_line = content.lines().next().unwrap_or("").trim().to_owned();
    (1, first_line)
}

/// Step 1: Collect symbols from resolved paths (pinned files and candidate paths).
fn collect_pinned_candidates(
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
fn search_terms_for(query: &str, parsed: &ParsedExploreQuery) -> Vec<String> {
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
fn score_matching_terms(
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
fn boost_route_intent_symbols(file_candidates: &mut FileCandidates) {
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

#[derive(Debug, Clone)]
struct RankedFileEntry {
    repo: String,
    path: String,
    constraint_match: u8,
    exact_evidence: u8,
    pinned_evidence: u8,
    structured_score: f64,
    content_score: f64,
    loose_score: f64,
    match_kind: Option<FileMatchKind>,
}

/// Step 3: Compute deterministic lexicographic ranking for files.
#[allow(clippy::too_many_lines)]
fn rank_all_files(
    file_candidates: &FileCandidates,
    pinned_files: &PinnedFiles,
    content_hits: &HashMap<(String, String), ContentHitInfo>,
    parsed: &ParsedExploreQuery,
    target_repo: Option<&str>,
) -> Vec<RankedFileEntry> {
    let mut all_keys = std::collections::HashSet::new();
    for key in file_candidates.keys() {
        all_keys.insert(key.clone());
    }
    for key in content_hits.keys() {
        all_keys.insert(key.clone());
    }
    for res_path in &parsed.resolved_paths {
        for (_, r, p) in res_path.files() {
            all_keys.insert((r, p));
        }
    }

    let asks_for_tests = parsed.search_terms.iter().any(|term| {
        let term = term.to_ascii_lowercase();
        term.starts_with("test") || term.starts_with("spec")
    });

    let mut ranked = Vec::new();

    for (repo, path) in all_keys {
        // Check constraint match
        let constraint_match = if let Some(tr) = target_repo {
            if repo == tr { 1 } else { 0 }
        } else {
            1
        };

        // Determine exact evidence & pinned evidence & match kind
        let mut exact_evidence: u8 = 0;
        let mut pinned_evidence: u8 = 0;
        let mut match_kind: Option<FileMatchKind> = None;

        for res_path in &parsed.resolved_paths {
            match res_path {
                ResolvedPath::ExactFile {
                    repo: r, path: p, ..
                } => {
                    if &repo == r && &path == p {
                        exact_evidence = exact_evidence.max(3);
                        pinned_evidence = 1;
                        match_kind = Some(FileMatchKind::ExactPath);
                    }
                }
                ResolvedPath::WorkspaceRelative {
                    repo: r, path: p, ..
                } => {
                    if &repo == r && &path == p {
                        exact_evidence = exact_evidence.max(3);
                        pinned_evidence = 1;
                        match_kind = Some(FileMatchKind::ExactPath);
                    }
                }
                ResolvedPath::UnambiguousBasename {
                    repo: r, path: p, ..
                } => {
                    if &repo == r && &path == p {
                        exact_evidence = exact_evidence.max(2);
                        pinned_evidence = 1;
                        match_kind = Some(FileMatchKind::ExactBasename);
                    }
                }
                ResolvedPath::AmbiguousBasename { files } => {
                    if files.iter().any(|(_, r, p)| &repo == r && &path == p) {
                        exact_evidence = exact_evidence.max(1);
                        if match_kind.is_none() {
                            match_kind = Some(FileMatchKind::ExactBasename);
                        }
                    }
                }
                ResolvedPath::DirectorySubtree { files } => {
                    if files.iter().any(|(_, r, p)| &repo == r && &path == p) {
                        pinned_evidence = pinned_evidence.max(1);
                        if match_kind.is_none() {
                            match_kind = Some(FileMatchKind::PinnedPath);
                        }
                    }
                }
            }
        }

        if pinned_files
            .get(&(repo.clone(), path.clone()))
            .copied()
            .unwrap_or(false)
        {
            pinned_evidence = 1;
        }

        // Check exact symbol match in this file
        let sym_cands = file_candidates.get(&(repo.clone(), path.clone()));
        let mut structured_score = 0.0;

        if let Some(cands) = sym_cands {
            for c in cands {
                if parsed
                    .search_terms
                    .iter()
                    .any(|t| t == &c.symbol.name || t.eq_ignore_ascii_case(&c.symbol.name))
                {
                    exact_evidence = exact_evidence.max(2);
                }
                structured_score += c.score;
            }
        }

        let mut content_score = 0.0;
        if let Some(chit) = content_hits.get(&(repo.clone(), path.clone())) {
            // Rank from FTS: lower is better or negative; transform to positive score
            content_score = 100.0 - chit.rank.clamp(-100.0, 100.0);
            if match_kind.is_none() {
                match_kind = Some(FileMatchKind::Content);
            }
        }

        if !asks_for_tests && is_test_path(&path) {
            structured_score *= 0.5;
            content_score *= 0.5;
        }

        let loose_score = structured_score + content_score;
        let total_score = if pinned_evidence > 0 {
            10000.0 + loose_score
        } else {
            loose_score
        };

        ranked.push(RankedFileEntry {
            repo,
            path,
            constraint_match,
            exact_evidence,
            pinned_evidence,
            structured_score,
            content_score,
            loose_score: total_score,
            match_kind,
        });
    }

    // Sort by:
    // (constraint_match DESC, exact_evidence DESC, pinned_evidence DESC, structured_score DESC, content_score DESC, loose_score DESC, repo ASC, path ASC)
    ranked.sort_by(|a, b| {
        b.constraint_match
            .cmp(&a.constraint_match)
            .then_with(|| b.loose_score.total_cmp(&a.loose_score))
            .then_with(|| b.exact_evidence.cmp(&a.exact_evidence))
            .then_with(|| b.pinned_evidence.cmp(&a.pinned_evidence))
            .then_with(|| b.structured_score.total_cmp(&a.structured_score))
            .then_with(|| b.content_score.total_cmp(&a.content_score))
            .then_with(|| a.repo.cmp(&b.repo))
            .then_with(|| a.path.cmp(&b.path))
    });
    let floor = ranked
        .first()
        .map_or(0.0, |top| (top.loose_score * 0.25).max(0.0));
    let has_pinned_or_dir = parsed
        .resolved_paths
        .iter()
        .any(|r| r.is_pinned() || matches!(r, ResolvedPath::DirectorySubtree { .. }));
    if has_pinned_or_dir {
        ranked.retain(|file| file.pinned_evidence > 0);
    } else {
        ranked.retain(|file| file.loose_score >= floor);
    }

    ranked
}

/// Step 4: Collect symbols and file matches respecting limits.
fn collect_final_results(
    mut file_candidates: FileCandidates,
    content_hits: &HashMap<(String, String), ContentHitInfo>,
    ranked_files: Vec<RankedFileEntry>,
    shown_files: usize,
    max_per_file: usize,
) -> ExploreCandidates {
    let mut final_symbols = Vec::new();
    let mut final_files = Vec::new();
    let mut not_shown = Vec::new();

    for (rank, file_info) in ranked_files.into_iter().enumerate() {
        let key = (file_info.repo.clone(), file_info.path.clone());
        let sym_cands = file_candidates.remove(&key);

        if rank < shown_files {
            let mut has_symbols = false;
            if let Some(mut cands) = sym_cands
                && !cands.is_empty()
            {
                has_symbols = true;
                cands.sort_by(|a, b| b.score.total_cmp(&a.score));
                if final_symbols.len() < MAX_EXPLORE_CANDIDATES {
                    cands
                        .truncate((MAX_EXPLORE_CANDIDATES - final_symbols.len()).min(max_per_file));
                    cands.sort_by_key(|c| (c.symbol.span.start_line, c.symbol.span.start_col));
                    final_symbols.extend(cands.into_iter().map(|sc| SymbolLocation {
                        symbol: sc.symbol,
                        file_path: sc.file_path,
                    }));
                }
            }

            if let Some(hit) = content_hits.get(&key) {
                if final_files.len() < MAX_FILE_CANDIDATES {
                    final_files.push(FileMatch {
                        repo: hit.repo.clone(),
                        file_path: hit.path.clone(),
                        match_kind: file_info.match_kind.unwrap_or(FileMatchKind::Content),
                        start_line: hit.start_line,
                        excerpt: hit.excerpt.clone(),
                        content_truncated: hit.content_truncated,
                    });
                }
            } else if let Some(kind) = file_info.match_kind {
                if final_files.len() < MAX_FILE_CANDIDATES {
                    final_files.push(FileMatch {
                        repo: file_info.repo.clone(),
                        file_path: file_info.path.clone(),
                        match_kind: kind,
                        start_line: 1,
                        excerpt: String::new(),
                        content_truncated: false,
                    });
                }
            } else if !has_symbols && final_files.len() < MAX_FILE_CANDIDATES {
                final_files.push(FileMatch {
                    repo: file_info.repo.clone(),
                    file_path: file_info.path.clone(),
                    match_kind: FileMatchKind::Content,
                    start_line: 1,
                    excerpt: String::new(),
                    content_truncated: false,
                });
            }
        } else {
            // Not shown: only include files that have matching symbols
            if let Some(mut cands) = sym_cands
                && !cands.is_empty()
            {
                cands.truncate(MAX_SYMBOLS_PER_FILE);
                cands.sort_by_key(|c| (c.symbol.span.start_line, c.symbol.span.start_col));
                not_shown.push(FileMention {
                    repo: file_info.repo,
                    file_path: file_info.path,
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
    }

    ExploreCandidates {
        symbols: final_symbols,
        files: final_files,
        not_shown,
    }
}
