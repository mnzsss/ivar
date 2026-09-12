//! Symbol search: exact, prefix, and FTS5 lookup for `find_symbols`, plus path
//! pinning, weighted OR ranking, and per-file candidate limiting for explore.

pub mod candidate;
pub mod intent;
pub mod rank;
pub mod search;

pub use candidate::ScoredCandidate;
pub use intent::{ParsedExploreQuery, ResolvedPath, is_path_like, resolve_query_paths};
pub use rank::{
    ExploreCandidates, MAX_EXPLORE_CANDIDATES, MAX_SYMBOLS_PER_FILE, explore_find,
    explore_find_candidates, is_test_path,
};
pub use search::find_symbols;
