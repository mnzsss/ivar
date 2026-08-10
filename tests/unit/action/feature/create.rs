#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::hall::{self, InitInput};
use crate::error::Status;
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
fn create_makes_the_feature_directory_and_records_the_feature() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let report = create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.name.as_str(), "checkout");
    assert_eq!(report.value.branch.as_str(), "checkout");
    assert!(fs::is_file(&root.join(".ivar/features/checkout/feature.json")).unwrap());
}

#[test]
fn create_rejects_a_feature_that_already_exists() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
        },
    )
    .unwrap();

    let error = create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
        },
    )
    .unwrap_err();

    assert_eq!(error.status, Status::Blocked);
    assert_eq!(error.code, "feature.already_exists");
}

#[test]
fn create_outside_a_hall_is_blocked() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root);

    let failure = create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "hall.not_found");
}

#[test]
fn create_rejects_an_invalid_name() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let failure = create(
        &ctx,
        CreateInput {
            name: "../etc".to_owned(),
            branch: None,
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "name.not_a_segment");
}

/// The whole point of the option: `feat/login` is a fine branch and an
/// impossible feature name, so without this the branch is unreachable.
#[test]
fn an_explicit_branch_may_be_one_a_feature_name_could_not_spell() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let report = create(
        &ctx,
        CreateInput {
            name: "login".to_owned(),
            branch: Some("feat/login".to_owned()),
        },
    )
    .unwrap();

    assert_eq!(report.value.name.as_str(), "login");
    assert_eq!(report.value.branch.as_str(), "feat/login");
    assert!(fs::is_file(&root.join(".ivar/features/login/feature.json")).unwrap());
}

/// The branch is still validated — `--branch` is not a hole in the rules.
#[test]
fn an_explicit_branch_is_still_validated() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let failure = create(
        &ctx,
        CreateInput {
            name: "login".to_owned(),
            branch: Some("../etc".to_owned()),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
}

#[test]
fn the_human_surface_names_the_feature_branch_and_root() {
    let outcome = CreateOutcome {
        root: Utf8PathBuf::from("/hall"),
        name: FeatureName::new("checkout").unwrap(),
        branch: BranchName::new("checkout").unwrap(),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Created feature `checkout` (branch: checkout) in /hall\n"
    );
}
