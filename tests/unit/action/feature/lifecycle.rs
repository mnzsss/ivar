//! Unit tests for `crate::action::feature::lifecycle` — the shared
//! plan-frontmatter close-record seam.
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
use crate::action::Ctx;
use crate::action::feature::create::CreateInput;
use crate::action::feature::create::create as create_action;
use crate::action::hall::{self, InitInput};
use crate::test_support::hall_root;
use camino::Utf8PathBuf;

fn seeded_hall() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();
    create_action(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: None,
        },
    )
    .unwrap();
    (guard, root)
}

fn feature(layout: &Layout, name: &str) -> Feature {
    Feature::read(layout, &FeatureName::new(name).unwrap())
        .unwrap()
        .unwrap()
}

#[test]
fn read_close_is_none_until_an_outcome_is_recorded() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(&root);

    assert_eq!(
        read_close(&layout, &FeatureName::new("checkout").unwrap()).unwrap(),
        None
    );
}

#[test]
fn write_close_records_outcome_and_closed_at_and_reads_back() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(&root);
    let name = FeatureName::new("checkout").unwrap();

    let written = write_close(&layout, &name, PromotionOutcome::Delivered).unwrap();

    assert_eq!(written.outcome, "delivered");
    assert!(!written.closed_at.is_empty());
    assert_eq!(read_close(&layout, &name).unwrap().unwrap(), written);

    // The plan body survives byte-for-byte: there was none, and there is none.
    let plan = crate::infra::fs::read_text(&layout.plan_dir(&name).join("plan.md"))
        .unwrap()
        .unwrap();
    assert!(plan.contains("outcome: delivered"));
}

#[test]
fn known_outcome_parses_known_names_and_is_none_for_foreign_ones() {
    let delivered = CloseRecord {
        outcome: "delivered".to_owned(),
        closed_at: "t".to_owned(),
    };
    assert_eq!(
        delivered.known_outcome(),
        Some(PromotionOutcome::Delivered)
    );

    let integrated = CloseRecord {
        outcome: "integrated".to_owned(),
        closed_at: "t".to_owned(),
    };
    assert_eq!(
        integrated.known_outcome(),
        Some(PromotionOutcome::Integrated)
    );

    // An outcome written by another tool still reads as "already closed" —
    // the string is preserved — but does not classify.
    let foreign = CloseRecord {
        outcome: "shipped".to_owned(),
        closed_at: "t".to_owned(),
    };
    assert_eq!(foreign.known_outcome(), None);
}

#[test]
fn is_fully_integrated_is_true_only_for_the_integrated_outcome() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(&root);
    let name = FeatureName::new("checkout").unwrap();
    let checkout = feature(&layout, "checkout");

    assert!(!is_fully_integrated(&layout, &checkout).unwrap());

    write_close(&layout, &name, PromotionOutcome::Delivered).unwrap();
    assert!(!is_fully_integrated(&layout, &checkout).unwrap());

    // A second close record cannot replace the first; use a fresh feature.
    let (_guard2, root2) = seeded_hall();
    let layout2 = Layout::at(&root2);
    let name2 = FeatureName::new("checkout").unwrap();
    let checkout2 = feature(&layout2, "checkout");
    write_close(&layout2, &name2, PromotionOutcome::Integrated).unwrap();
    assert!(is_fully_integrated(&layout2, &checkout2).unwrap());
}
