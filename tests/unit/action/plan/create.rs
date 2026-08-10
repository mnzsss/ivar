#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::feature::create::{
    self as feature_create, CreateInput as FeatureCreateInput,
};
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
    feature_create::create(
        &ctx,
        FeatureCreateInput {
            name: "checkout".to_owned(),
            branch: None,
        },
    )
    .unwrap();
    (guard, root)
}

#[test]
fn create_scaffolds_the_three_artifacts() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let report = create(
        &ctx,
        CreateInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    let plan_dir = root.join("plans/checkout");
    assert!(fs::is_file(&plan_dir.join("requirements.md")).unwrap());
    assert!(fs::is_file(&plan_dir.join("analysis.md")).unwrap());
    assert!(fs::is_file(&plan_dir.join("plan.md")).unwrap());
}

#[test]
fn create_is_rejected_for_a_missing_feature() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let failure = create(
        &ctx,
        CreateInput {
            feature: "ghost".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.feature_not_found");
}

#[test]
fn create_is_rejected_when_artifacts_already_exist() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    create(
        &ctx,
        CreateInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let failure = create(
        &ctx,
        CreateInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.already_exists");
}

#[test]
fn the_human_surface_names_the_plan_dir() {
    let outcome = CreateOutcome {
        root: Utf8PathBuf::from("/hall"),
        feature: FeatureName::new("checkout").unwrap(),
        plan_dir: Utf8PathBuf::from("/hall/plans/checkout"),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Created SPDD artifacts for `checkout` in /hall/plans/checkout\n"
    );
}
