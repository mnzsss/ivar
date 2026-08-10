#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::feature::create::CreateInput;
use crate::action::feature::create::create as create_action;
use crate::action::hall::{self, InitInput};
use crate::test_support::hall_root;

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
    (guard, root)
}

#[test]
fn list_reports_an_empty_hall_as_empty() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let report = list(&ctx).unwrap();

    assert!(report.is_clean());
    assert!(report.value.features.is_empty());
}

#[test]
fn list_reports_created_features_sorted_by_name() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    create_action(
        &ctx,
        CreateInput {
            name: "zeta".to_owned(),
            branch: None,
        },
    )
    .unwrap();
    create_action(
        &ctx,
        CreateInput {
            name: "alpha".to_owned(),
            branch: None,
        },
    )
    .unwrap();

    let report = list(&ctx).unwrap();

    let names: Vec<&str> = report
        .value
        .features
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, vec!["alpha", "zeta"]);
    assert_eq!(report.value.features[0].promoted_count, 0);
}

#[test]
fn list_outside_a_hall_is_blocked() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root);

    let failure = list(&ctx).unwrap_err();

    assert_eq!(failure.code, "hall.not_found");
}

#[test]
fn the_human_surface_lists_features_with_their_counts() {
    let outcome = ListOutcome {
        root: Utf8PathBuf::from("/hall"),
        features: vec![FeatureSummary {
            name: FeatureName::new("checkout").unwrap(),
            branch: "checkout".to_owned(),
            promoted_count: 2,
            ready_count: 1,
        }],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Features in /hall:\n  checkout  branch checkout  promoted 1/2\n"
    );
}
