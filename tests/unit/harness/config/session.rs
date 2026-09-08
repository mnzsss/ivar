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
    let block = build_session_block(&feature(), "../../plan.md");

    assert!(
        block.contains("ivar session — feature `checkout`"),
        "was: {block}"
    );
    assert!(
        block.contains("ivar plan status ../../plan.md"),
        "was: {block}"
    );
    assert!(
        block.contains("../../requirements.md")
            && block.contains("../../analysis.md")
            && block.contains("../../plan.md"),
        "was: {block}"
    );
    assert!(block.contains("needs-revision"), "was: {block}");
    assert!(
        !block.contains("plans/checkout"),
        "stale plans/ path: {block}"
    );
}

#[test]
fn the_block_describes_receipt_recovery_for_every_state() {
    let block = build_session_block(&feature(), "../../plan.md");

    for required in [
        "ivar feature execute status checkout",
        "No receipt or a terminal receipt",
        "active` or `blocked",
        "--resume",
        "diverged",
        "accept-revision checkout --plan ../../plan.md",
        "--restart",
    ] {
        assert!(block.contains(required), "missing `{required}`: {block}");
    }

    for removed in ["execution board", "journal", "workstream", "tick", "plans/"] {
        assert!(!block.contains(removed), "stale `{removed}`: {block}");
    }
}

#[test]
fn building_the_same_block_twice_produces_identical_bytes() {
    let first = build_session_block(&feature(), "../../plan.md");
    let second = build_session_block(&feature(), "../../plan.md");

    assert_eq!(first, second);
}

#[test]
fn the_block_depends_on_its_feature_and_plan_path() {
    let checkout = build_session_block(&feature(), "../../plan.md");
    let other = build_session_block(&FeatureName::new("web").unwrap(), "../../plan.md");

    assert_ne!(checkout, other);
    assert!(other.contains("feature `web`"));
    assert!(other.contains("../../plan.md"));
}

#[test]
fn compose_instructions_appends_memory_when_markers_absent() {
    let base = "# Hall standing instructions\n\n- rule 1";
    let memory = "<!-- ivar:memory:start -->\n## Shared Memory\n<!-- ivar:memory:end -->";
    let composed = compose_instructions_with_memory(base, memory);
    assert_eq!(composed, format!("{base}\n\n{memory}"));
}

#[test]
fn compose_instructions_preserves_content_outside_markers_byte_exact() {
    let before = "# Prefix content\n\n";
    let old_memory = "<!-- ivar:memory:start -->\nOld content\n<!-- ivar:memory:end -->";
    let after = "\n\n# Suffix content\n- do not modify";
    let full = format!("{before}{old_memory}{after}");

    let new_memory =
        "<!-- ivar:memory:start -->\nNew shared memory content\n<!-- ivar:memory:end -->";
    let composed = compose_instructions_with_memory(&full, new_memory);

    assert_eq!(composed, format!("{before}{new_memory}{after}"));
}

#[test]
fn compose_instructions_with_empty_memory_is_noop_or_removes_markers() {
    let base = "# Hall instructions";
    assert_eq!(compose_instructions_with_memory(base, ""), base);

    let with_markers = "Header\n<!-- ivar:memory:start -->content<!-- ivar:memory:end -->\nFooter";
    assert_eq!(
        compose_instructions_with_memory(with_markers, ""),
        "Header\n\nFooter"
    );
}
