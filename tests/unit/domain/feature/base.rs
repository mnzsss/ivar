//! Unit tests for `crate::domain::feature::base`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

#[test]
fn a_declared_base_wins_over_the_default_branch() {
    let declared = BranchName::new("feat/custom-base").unwrap();
    let default_branch = BranchName::new("main").unwrap();

    let base = effective_base(Some(&declared), &default_branch);

    assert_eq!(base, declared);
}

#[test]
fn no_declared_base_falls_back_to_the_default_branch() {
    let default_branch = BranchName::new("main").unwrap();

    let base = effective_base(None, &default_branch);

    assert_eq!(base, default_branch);
}
