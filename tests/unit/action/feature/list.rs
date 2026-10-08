#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::feature::create::CreateInput;
use crate::action::feature::create::create as create_action;
use crate::test_support::{hall_root, seeded_hall};

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
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    create_action(
        &ctx,
        CreateInput {
            name: "alpha".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
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
            parent: None,
            depth: 0,
            state: crate::domain::feature::FeatureIntegrationState::Active,
            blockers: Vec::new(),
            repos: Vec::new(),
        }],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    let stripped = anstream::adapter::strip_str(&String::from_utf8(out).unwrap()).to_string();

    assert_eq!(
        stripped,
        "Features in /hall:\nFEATURE   BRANCH    PROMOTED  STATE\ncheckout  checkout  1/2       active\n"
    );
}

#[test]
fn list_orders_features_as_forest_and_renders_glyphs_in_human_output() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    // Root "alpha" with child "zz" — child sorts after root "beta", so
    // alphabetical order would be [alpha, alpha-sub, beta, zz], but forest
    // order is [alpha, zz, beta].
    create_action(
        &ctx,
        CreateInput {
            name: "alpha".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    create_action(
        &ctx,
        CreateInput {
            name: "zz".to_owned(),
            branch: None,
            base: None,
            parent: Some("alpha".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    create_action(
        &ctx,
        CreateInput {
            name: "beta".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let report = list(&ctx).unwrap();

    // Forest order: alpha, zz (child of alpha), beta — not alphabetical
    let names: Vec<&str> = report
        .value
        .features
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, vec!["alpha", "zz", "beta"]);

    // Verify human output renders glyphs in the FEATURE column
    let mut output = Vec::new();
    report.value.write_human(&mut output).unwrap();
    let rendered = String::from_utf8(output).unwrap();

    // "alpha" is a root → no prefix
    assert!(rendered.contains("alpha"));
    // "zz" is the last (and only) child of "alpha" → "└── zz"
    assert!(rendered.contains("└── zz"));
    // "beta" is a root → no prefix
    assert!(rendered.contains("beta"));
}

#[test]
fn list_skips_cycle_members_but_still_lists_dangling_parent_root() {
    use crate::domain::name::BranchName;
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root);

    // Write features using Feature::new + .write(&layout) which creates directories
    // - "dangling" has parent "absent" (not in map) → listed at depth 0 as root
    // - "child-of-dangling" has parent "dangling" → listed at depth 1 beneath dangling
    // - "cyc-a" and "cyc-b" name each other as parent → form a cycle and are absent
    let mut dangling = Feature::new(
        FeatureName::new("dangling").unwrap(),
        BranchName::new("dangling").unwrap(),
    );
    dangling.parent = Some(FeatureName::new("absent").unwrap());
    dangling.write(&layout).unwrap();

    let mut child = Feature::new(
        FeatureName::new("child-of-dangling").unwrap(),
        BranchName::new("child-of-dangling").unwrap(),
    );
    child.parent = Some(FeatureName::new("dangling").unwrap());
    child.write(&layout).unwrap();

    let mut cyc_a = Feature::new(
        FeatureName::new("cyc-a").unwrap(),
        BranchName::new("cyc-a").unwrap(),
    );
    cyc_a.parent = Some(FeatureName::new("cyc-b").unwrap());
    cyc_a.write(&layout).unwrap();

    let mut cyc_b = Feature::new(
        FeatureName::new("cyc-b").unwrap(),
        BranchName::new("cyc-b").unwrap(),
    );
    cyc_b.parent = Some(FeatureName::new("cyc-a").unwrap());
    cyc_b.write(&layout).unwrap();

    let report = list(&ctx).unwrap();
    let names_and_depths: Vec<(&str, usize)> = report
        .value
        .features
        .iter()
        .map(|f| (f.name.as_str(), f.depth))
        .collect();

    // Dangling feature listed at depth 0 as root; child beneath it at depth 1; cycle members absent
    assert_eq!(
        names_and_depths,
        vec![("dangling", 0), ("child-of-dangling", 1)]
    );
}
