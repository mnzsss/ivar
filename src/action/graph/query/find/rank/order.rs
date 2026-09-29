use std::collections::{HashMap, HashSet};

use super::{ContentHitInfo, FileCandidates, PinnedFiles, RankedFileEntry, is_test_path};
use crate::action::graph::query::find::intent::{
    ParsedExploreQuery, ResolvedPath, is_identifier_shaped,
};
use crate::domain::graph::FileMatchKind;

/// Step 3: Compute deterministic lexicographic ranking for files.
#[allow(clippy::too_many_lines)]
pub(super) fn rank_all_files(
    file_candidates: &FileCandidates,
    pinned_files: &PinnedFiles,
    content_hits: &HashMap<(String, String), ContentHitInfo>,
    parsed: &ParsedExploreQuery,
    target_repos: &[String],
) -> Vec<RankedFileEntry> {
    let mut all_keys = HashSet::new();
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
        let constraint_match =
            if target_repos.is_empty() || target_repos.iter().any(|tr| tr == &repo) {
                1
            } else {
                0
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
                    .any(|t| is_identifier_shaped(t) && t == &c.symbol.name)
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
            .then_with(|| b.exact_evidence.cmp(&a.exact_evidence))
            .then_with(|| b.pinned_evidence.cmp(&a.pinned_evidence))
            .then_with(|| b.loose_score.total_cmp(&a.loose_score))
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
        ranked.retain(|file| file.exact_evidence >= 2 || file.loose_score >= floor);
    }

    ranked
}
