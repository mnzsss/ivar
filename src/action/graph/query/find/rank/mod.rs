//! Explore ranking: collect pinned and scored symbol candidates, add file-content
//! hits, order files lexicographically, then fill the combined result budget.

mod content;
mod coverage;
mod order;
mod score;
mod select;

use std::collections::{BTreeSet, HashMap};

use super::candidate::ScoredCandidate;
use super::intent::{ResolvedPath, is_identifier_shaped, resolve_query_paths};
use crate::action::graph::query::types::{QueryError, SymbolLocation};
use crate::domain::graph::{
    FileMatch, FileMatchKind, FileMention, MentionedSymbol, WeakMatch, WeakMatches,
};
use crate::store::graph::db::GraphDb;
use content::search_content_hits;
use coverage::{FileKey, QueryTerms, TermCoverage, path_names_term};
use order::rank_all_files;
use score::{
    boost_route_intent_symbols, collect_pinned_candidates, score_matching_terms, search_terms_for,
};
use select::collect_final_results;

type FileCandidates = HashMap<(String, String), Vec<ScoredCandidate>>;
type PinnedFiles = HashMap<(String, String), bool>;

/// Symbols and files an explore answer shows source for, and the matching files it leaves out.
#[derive(Debug, Clone, Default)]
pub struct ExploreCandidates {
    pub symbols: Vec<SymbolLocation>,
    pub files: Vec<FileMatch>,
    pub not_shown: Vec<FileMention>,
    /// Set when no file matched an intent query well enough; the other fields are then empty.
    pub weak: Option<WeakMatches>,
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

/// Maximum candidates returned for hero explore (combined symbols + matched files).
pub const MAX_EXPLORE_CANDIDATES: usize = 20;
pub const MAX_SYMBOLS_PER_FILE: usize = 6;
/// Maximum file matches returned for exploration.
pub const MAX_FILE_CANDIDATES: usize = 6;
/// Most weak candidates named when an intent query has no confident match.
const MAX_WEAK_FILES: usize = 5;
/// Most symbols named per weak candidate.
const MAX_WEAK_SYMBOLS: usize = 3;

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
    /// Lowercased query terms found in this file's content.
    terms: BTreeSet<String>,
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
    let single_repo_scope = if parsed.target_repos.len() == 1 {
        parsed.target_repos.first().map(String::as_str)
    } else {
        repo
    };
    let conn = db.conn();

    let (mut file_candidates, pinned_files) = collect_pinned_candidates(conn, &parsed)?;

    let terms_to_search = search_terms_for(query, &parsed);
    let symbol_coverage = score_matching_terms(
        conn,
        &terms_to_search,
        single_repo_scope,
        parsed.route_intent,
        &mut file_candidates,
    )?;

    if parsed.route_intent {
        boost_route_intent_symbols(&mut file_candidates);
    }

    // Also collect file content matches
    let content_hits = search_content_hits(db, &terms_to_search, single_repo_scope)?;

    let query_terms = QueryTerms::new(&terms_to_search);
    let names_files = parsed
        .resolved_paths
        .iter()
        .any(|r| r.is_pinned() || matches!(r, ResolvedPath::DirectorySubtree { .. }));
    // Intent queries are prose: no path named, no identifier spelled out.
    let prefer_docs = !names_files
        && query_terms.required() > 0
        && !terms_to_search.iter().any(|t| is_identifier_shaped(t));

    let mut ranked_files = rank_all_files(
        &file_candidates,
        &pinned_files,
        &content_hits,
        &parsed,
        &parsed.target_repos,
        prefer_docs,
    );
    if ranked_files.is_empty() {
        return Ok(ExploreCandidates::default());
    }

    if !names_files && query_terms.required() > 0 {
        let coverage = file_coverage(&symbol_coverage, &content_hits, &ranked_files, &query_terms);
        let required = query_terms.required();
        let (strong, weak): (Vec<_>, Vec<_>) = ranked_files.into_iter().partition(|file| {
            file.exact_evidence >= 2
                || coverage
                    .get(&(file.repo.clone(), file.path.clone()))
                    .is_some_and(|terms| terms.len() >= required)
        });
        if strong.is_empty() {
            return Ok(ExploreCandidates {
                weak: Some(weak_matches(
                    weak,
                    &file_candidates,
                    &coverage,
                    &symbol_coverage,
                    &content_hits,
                    &query_terms,
                )),
                ..ExploreCandidates::default()
            });
        }
        ranked_files = strong;
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

/// Query terms each ranked file matches, through its symbols, content, or path.
fn file_coverage(
    symbol_coverage: &TermCoverage,
    content_hits: &HashMap<FileKey, ContentHitInfo>,
    ranked_files: &[RankedFileEntry],
    terms: &QueryTerms,
) -> TermCoverage {
    ranked_files
        .iter()
        .map(|file| {
            let key = (file.repo.clone(), file.path.clone());
            let mut matched: BTreeSet<String> = BTreeSet::new();
            matched.extend(symbol_coverage.get(&key).into_iter().flatten().cloned());
            matched.extend(
                content_hits
                    .get(&key)
                    .into_iter()
                    .flat_map(|h| h.terms.iter().cloned()),
            );
            matched.extend(
                terms
                    .keys()
                    .filter(|term| path_names_term(&file.path, term))
                    .map(str::to_owned),
            );
            matched.retain(|term| terms.contains(term));
            (key, matched)
        })
        .collect()
}

/// The best-covered files as weak candidates, and the terms nothing matched.
fn weak_matches(
    mut ranked_files: Vec<RankedFileEntry>,
    file_candidates: &FileCandidates,
    coverage: &TermCoverage,
    symbol_coverage: &TermCoverage,
    content_hits: &HashMap<FileKey, ContentHitInfo>,
    terms: &QueryTerms,
) -> WeakMatches {
    let covered = |file: &RankedFileEntry| {
        coverage
            .get(&(file.repo.clone(), file.path.clone()))
            .map_or(0, BTreeSet::len)
    };
    // Stable: files equally covered keep their rank order.
    ranked_files.sort_by_key(|file| std::cmp::Reverse(covered(file)));

    let mut anywhere: BTreeSet<&str> = BTreeSet::new();
    anywhere.extend(symbol_coverage.values().flatten().map(String::as_str));
    anywhere.extend(
        content_hits
            .values()
            .flat_map(|h| h.terms.iter().map(String::as_str)),
    );

    let candidates = ranked_files
        .into_iter()
        .take(MAX_WEAK_FILES)
        .map(|file| {
            let key = (file.repo.clone(), file.path.clone());
            let mut symbols = file_candidates.get(&key).cloned().unwrap_or_default();
            symbols.sort_by(|a, b| b.score.total_cmp(&a.score));
            symbols.truncate(MAX_WEAK_SYMBOLS);
            symbols.sort_by_key(|c| (c.symbol.span.start_line, c.symbol.span.start_col));
            WeakMatch {
                matched_terms: coverage
                    .get(&key)
                    .into_iter()
                    .flatten()
                    .map(|term| terms.display(term))
                    .collect(),
                symbols: symbols
                    .into_iter()
                    .map(|c| MentionedSymbol {
                        name: c.symbol.name,
                        line: c.symbol.span.start_line,
                    })
                    .collect(),
                repo: file.repo,
                file_path: file.path,
            }
        })
        .collect();

    WeakMatches {
        total_terms: terms.total(),
        required_terms: terms.required(),
        unmatched_terms: terms
            .keys()
            .filter(|term| !anywhere.contains(term))
            .map(|term| terms.display(term))
            .collect(),
        candidates,
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
