#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::name::FeatureName;

fn feature() -> FeatureName {
    FeatureName::new("checkout").unwrap()
}

#[test]
fn the_block_names_the_feature_and_how_to_re_derive_planning_state() {
    let block = build_session_block(&feature(), "plans/checkout/plan.md");

    assert!(
        block.contains("ivar session — feature `checkout`"),
        "was: {block}"
    );
    assert!(
        block.contains("ivar plan status plans/checkout/plan.md"),
        "was: {block}"
    );
    assert!(
        block.contains("requirements.md")
            && block.contains("analysis.md")
            && block.contains("plan.md"),
        "was: {block}"
    );
    assert!(block.contains("needs-revision"), "was: {block}");
}

#[test]
fn the_block_describes_receipt_recovery_for_every_state() {
    let block = build_session_block(&feature(), "plans/checkout/plan.md");

    for required in [
        "ivar feature execute status checkout",
        "No receipt or a terminal receipt",
        "active` or `blocked",
        "--resume",
        "diverged",
        "accept-revision checkout --plan plans/checkout/plan.md",
        "--restart",
    ] {
        assert!(block.contains(required), "missing `{required}`: {block}");
    }

    for removed in ["execution board", "journal", "workstream", "tick"] {
        assert!(!block.contains(removed), "stale `{removed}`: {block}");
    }
}

#[test]
fn building_the_same_block_twice_produces_identical_bytes() {
    let first = build_session_block(&feature(), "plans/checkout/plan.md");
    let second = build_session_block(&feature(), "plans/checkout/plan.md");

    assert_eq!(first, second);
}

#[test]
fn the_block_depends_on_its_feature_and_plan_path() {
    let checkout = build_session_block(&feature(), "plans/checkout/plan.md");
    let other = build_session_block(&FeatureName::new("web").unwrap(), "plans/web/plan.md");

    assert_ne!(checkout, other);
    assert!(other.contains("feature `web`"));
    assert!(other.contains("plans/web/plan.md"));
}
