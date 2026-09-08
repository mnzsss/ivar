//! Domain models for shared memory query and reconciliation.

use serde::{Deserialize, Serialize};

use super::config::ScopeName;

/// Filter criteria for querying the memory index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryFilter {
    /// Optional scope name to restrict search results to a specific scope.
    pub scope: Option<ScopeName>,
    /// Maximum number of search results to return.
    pub limit: usize,
}

impl Default for QueryFilter {
    fn default() -> Self {
        Self {
            scope: None,
            limit: 20,
        }
    }
}

/// A matched search result returned from the memory index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryMatch {
    /// Scope this topic belongs to.
    pub scope: ScopeName,
    /// Topic slug.
    pub slug: String,
    /// Path to the markdown document.
    pub path: String,
    /// Topic title.
    pub title: String,
    /// Snippet preview with matched terms.
    pub snippet: String,
    /// Relevance rank (e.g. SQLite BM25 score).
    pub rank: f64,
}

/// Summary of an index reconciliation run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReconcileSummary {
    /// Number of newly indexed topic documents.
    pub indexed: usize,
    /// Number of existing topic documents updated due to content changes.
    pub updated: usize,
    /// Number of deleted topic documents removed from the index.
    pub removed: usize,
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/memory/query.rs"]
mod tests;
