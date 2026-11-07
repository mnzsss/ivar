//! `ivar memory`: the hall memory database's verbs.

pub mod brief;
pub mod capture;
pub mod classify;
pub mod clean;
pub mod consolidate;
pub mod coordination;
pub mod delivery;
pub mod drain;
pub mod events;
pub mod lint;
pub mod mcp;
pub mod page_file;
pub mod query;
pub mod reindex;
pub mod review;
pub mod status;
pub mod sweep;

#[cfg(test)]
#[path = "../../../tests/unit/action/memory/mod.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../tests/unit/action/memory/provider_matrix.rs"]
mod provider_matrix;

#[cfg(test)]
#[path = "../../../tests/unit/action/memory/review.rs"]
mod review_tests;
