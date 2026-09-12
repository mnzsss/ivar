use std::collections::HashMap;

use crate::domain::graph::Symbol;

/// Candidate symbol associated with its file path and search score.
#[derive(Debug, Clone)]
pub struct ScoredCandidate {
    pub symbol: Symbol,
    pub file_path: String,
    pub score: f64,
}

/// Adds `score` to a symbol's candidate entry, creating the entry on first sight.
pub(crate) fn add_score(
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
