use std::collections::HashMap;

use super::{
    ContentHitInfo, ExploreCandidates, FileCandidates, MAX_EXPLORE_CANDIDATES, MAX_FILE_CANDIDATES,
    MAX_SYMBOLS_PER_FILE, RankedFileEntry,
};
use crate::action::graph::query::types::SymbolLocation;
use crate::domain::graph::{FileMatch, FileMatchKind, FileMention, MentionedSymbol};

/// Step 4: Collect symbols and file matches respecting limits.
pub(super) fn collect_final_results(
    mut file_candidates: FileCandidates,
    content_hits: &HashMap<(String, String), ContentHitInfo>,
    ranked_files: Vec<RankedFileEntry>,
    shown_files: usize,
    max_per_file: usize,
) -> ExploreCandidates {
    let mut final_symbols = Vec::new();
    let mut final_files = Vec::new();
    let mut not_shown = Vec::new();

    // Pass 1: Reserve files for pinned/exact or content hits first
    for (rank, file_info) in ranked_files.iter().enumerate() {
        if rank >= shown_files {
            break;
        }
        let key = (file_info.repo.clone(), file_info.path.clone());
        let has_symbols = file_candidates.get(&key).is_some_and(|c| !c.is_empty());

        if final_files.len() < MAX_FILE_CANDIDATES
            && (final_files.len() + final_symbols.len()) < MAX_EXPLORE_CANDIDATES
        {
            if let Some(hit) = content_hits.get(&key) {
                final_files.push(FileMatch {
                    repo: hit.repo.clone(),
                    file_path: hit.path.clone(),
                    match_kind: file_info
                        .match_kind
                        .clone()
                        .unwrap_or(FileMatchKind::Content),
                    start_line: hit.start_line,
                    excerpt: hit.excerpt.clone(),
                    content_truncated: hit.content_truncated,
                });
            } else if let Some(kind) = &file_info.match_kind {
                if !has_symbols {
                    final_files.push(FileMatch {
                        repo: file_info.repo.clone(),
                        file_path: file_info.path.clone(),
                        match_kind: kind.clone(),
                        start_line: 1,
                        excerpt: String::new(),
                        content_truncated: false,
                    });
                }
            } else if !has_symbols {
                final_files.push(FileMatch {
                    repo: file_info.repo.clone(),
                    file_path: file_info.path.clone(),
                    match_kind: FileMatchKind::Content,
                    start_line: 1,
                    excerpt: String::new(),
                    content_truncated: false,
                });
            }
        }
    }

    // Pass 2: Collect symbols up to remaining budget
    for (rank, file_info) in ranked_files.into_iter().enumerate() {
        let key = (file_info.repo.clone(), file_info.path.clone());
        let sym_cands = file_candidates.remove(&key);

        if rank < shown_files {
            if let Some(mut cands) = sym_cands
                && !cands.is_empty()
            {
                cands.sort_by(|a, b| b.score.total_cmp(&a.score));
                let remaining_budget =
                    MAX_EXPLORE_CANDIDATES.saturating_sub(final_symbols.len() + final_files.len());
                if remaining_budget > 0 {
                    cands.truncate(remaining_budget.min(max_per_file));
                    cands.sort_by_key(|c| (c.symbol.span.start_line, c.symbol.span.start_col));
                    final_symbols.extend(cands.into_iter().map(|sc| SymbolLocation {
                        symbol: sc.symbol,
                        file_path: sc.file_path,
                    }));
                }
            }
        } else if let Some(mut cands) = sym_cands
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

    ExploreCandidates {
        symbols: final_symbols,
        files: final_files,
        not_shown,
    }
}
