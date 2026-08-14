//! Unit tests for `crate::domain::feature::promotion` (`Feature`, `Promotion`).
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
use crate::domain::name::{BranchName, FeatureName, RepoName};

fn feature() -> Feature {
    Feature::new(
        FeatureName::new("checkout").unwrap(),
        BranchName::new("feat/checkout").unwrap(),
    )
}

#[test]
fn a_new_feature_has_no_declared_base() {
    assert_eq!(feature().base, None);
}

#[test]
fn a_new_promotion_has_no_declared_base() {
    let mut feature = feature();
    let repo = RepoName::new("api").unwrap();

    feature.promote(repo.clone());

    assert_eq!(feature.promotions.get(&repo).unwrap().base, None);
}

#[test]
fn a_feature_is_stamped_at_version_two() {
    assert_eq!(feature().version(), 2);
}

#[test]
fn base_round_trips_through_serde_when_set() {
    let mut feature = feature();
    feature.base = Some(BranchName::new("main").unwrap());

    let rendered = serde_json::to_string(&feature).unwrap();
    let parsed: Feature = serde_json::from_str(&rendered).unwrap();

    assert_eq!(parsed, feature);
    assert_eq!(parsed.base, Some(BranchName::new("main").unwrap()));
}

#[test]
fn a_v1_feature_json_with_no_base_field_still_deserialises() {
    let raw = r#"{"version":1,"name":"checkout","branch":"feat/checkout","promotions":{}}"#;

    let parsed: Feature = serde_json::from_str(raw).unwrap();

    assert_eq!(parsed.base, None);
}

#[test]
fn a_v1_promotion_with_no_base_field_still_deserialises() {
    let raw = r#"{"version":1,"name":"checkout","branch":"feat/checkout","promotions":{"api":{"worktree":"pending"}}}"#;

    let parsed: Feature = serde_json::from_str(raw).unwrap();

    assert_eq!(
        parsed
            .promotions
            .get(&RepoName::new("api").unwrap())
            .unwrap()
            .base,
        None
    );
}
