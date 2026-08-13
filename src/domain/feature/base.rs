//! The effective base branch for a feature's promotions.
//!
//! A feature may declare a `base` branch explicitly (`Feature::base` or a
//! per-repo `Promotion::base`); when it does not, the repo's own
//! `default_branch` from `ivar.json` stands in. [`effective_base`] is the
//! one place that choice is made — pure, no I/O, importing only from
//! `domain` so it can be called from `action` without pulling `store` in
//! along with it.

use super::super::name::BranchName;

/// The branch a promotion should be based on: `declared` if the feature (or
/// promotion) named one explicitly, otherwise `default_branch`.
#[must_use]
pub fn effective_base(declared: Option<&BranchName>, default_branch: &BranchName) -> BranchName {
    declared.cloned().unwrap_or_else(|| default_branch.clone())
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/feature/base.rs"]
mod tests;
