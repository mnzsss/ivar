#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
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
            base: None,
        },
    )
    .unwrap();
    plan_create::create(
        &ctx,
        PlanCreateInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    (guard, root)
}

#[test]
fn show_prints_the_scaffolded_artifact() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let report = show(
        &ctx,
        ShowInput {
            feature: "checkout".to_owned(),
            artifact: Artifact::Requirements,
        },
    )
    .unwrap();

    assert!(report.value.content.contains("# Requirements"));
}

#[test]
fn show_is_rejected_for_a_missing_artifact() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    // Delete the artifact behind ivar's back.
    fs::remove_path(&root.join("plans/checkout/analysis.md")).unwrap();

    let failure = show(
        &ctx,
        ShowInput {
            feature: "checkout".to_owned(),
            artifact: Artifact::Analysis,
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.artifact_missing");
}

#[test]
fn artifact_filenames_match_the_layout_contract() {
    assert_eq!(Artifact::Requirements.filename(), "requirements.md");
    assert_eq!(Artifact::Analysis.filename(), "analysis.md");
    assert_eq!(Artifact::Plan.filename(), "plan.md");
}
