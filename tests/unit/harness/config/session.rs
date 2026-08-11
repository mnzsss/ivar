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

/// The block is the continuation contract: it names the feature and says how
/// to re-derive the SPDD stage — the status command first, then the artifacts.
#[test]
fn the_block_names_the_feature_and_how_to_re_derive_the_stage() {
    let block = build_session_block(&feature(), "plans/checkout/plan.md");

    assert!(
        block.contains("ivar session — feature `checkout`"),
        "the block must name the feature: {block}"
    );
    assert!(
        block.contains("ivar plan status plans/checkout/plan.md"),
        "the block must tell the agent to run plan status: {block}"
    );
    assert!(
        block.contains("requirements.md") && block.contains("analysis.md") && block.contains("plan.md"),
        "the block must name the plan artifacts: {block}"
    );
    assert!(
        block.contains("needs-revision"),
        "the block must say what a needs-revision gate means: {block}"
    );
}

/// The builder is pure: identical inputs produce identical bytes, which is
/// what lets materialisation decide "unchanged" by comparison.
#[test]
fn building_the_same_block_twice_produces_identical_bytes() {
    let first = build_session_block(&feature(), "plans/checkout/plan.md");
    let second = build_session_block(&feature(), "plans/checkout/plan.md");

    assert_eq!(first, second);
}

/// The block is a function of its arguments: a different feature or plan path
/// produces different bytes, so a relay to another feature can never inherit
/// the previous feature's continuation contract.
#[test]
fn the_block_depends_on_its_feature_and_plan_path() {
    let checkout = build_session_block(&feature(), "plans/checkout/plan.md");
    let other = build_session_block(&FeatureName::new("web").unwrap(), "plans/web/plan.md");

    assert_ne!(checkout, other);
    assert!(other.contains("feature `web`"));
    assert!(other.contains("plans/web/plan.md"));
}
