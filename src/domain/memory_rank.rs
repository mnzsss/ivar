//! Reciprocal-rank fusion constants and the FTS5 boundary for user text.

use super::memory_page::Tier;

pub const RRF_K: f64 = 60.0;
pub const NEIGHBOUR_SEEDS: usize = 10;
pub const PINNED_BONUS: f64 = 1.1;

#[must_use]
pub const fn tier_multiplier(tier: Tier) -> f64 {
    match tier {
        Tier::Semantic => 1.05,
        Tier::Procedural => 1.0,
        Tier::Episodic | Tier::Working => 0.9,
    }
}

#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn rrf_contribution(rank: usize) -> f64 {
    1.0 / (RRF_K + rank as f64 + 1.0)
}

/// Every token is quoted, so no input reaches the FTS5 parser as syntax.
#[must_use]
pub fn fts_expression(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split_whitespace()
        .map(|term| term.replace('"', ""))
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{term}\""))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

#[cfg(test)]
#[path = "../../tests/unit/domain/memory_rank.rs"]
mod tests;
