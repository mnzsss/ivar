#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
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
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    (guard, root)
}

#[test]
fn create_scaffolds_the_three_artifacts() {
    // R-SCAFFOLD-DEFAULT: no artifact list -> all three, unchanged.
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let report = create(
        &ctx,
        CreateInput {
            feature: "checkout".to_owned(),
            artifacts: Vec::new(),
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
            artifacts: Vec::new(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.feature_not_found");
}

#[test]
fn create_is_rejected_when_artifacts_already_exist_and_no_subset_was_named() {
    // N-BACKWARD-COMPATIBLE: the no-list path still refuses on ANY existing
    // artifact, exactly as before the subset option existed.
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    create(
        &ctx,
        CreateInput {
            feature: "checkout".to_owned(),
            artifacts: Vec::new(),
        },
    )
    .unwrap();

    let failure = create(
        &ctx,
        CreateInput {
            feature: "checkout".to_owned(),
            artifacts: Vec::new(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.already_exists");
}

#[test]
fn create_with_a_named_subset_writes_only_that_artifact() {
    // R-SCAFFOLD-SUBSET: `create <f> plan` writes only plan.md.
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let report = create(
        &ctx,
        CreateInput {
            feature: "checkout".to_owned(),
            artifacts: vec![Artifact::Plan],
        },
    )
    .unwrap();

    assert!(report.is_clean());
    let plan_dir = root.join("plans/checkout");
    assert!(fs::is_file(&plan_dir.join("plan.md")).unwrap());
    assert!(!fs::is_file(&plan_dir.join("requirements.md")).unwrap());
    assert!(!fs::is_file(&plan_dir.join("analysis.md")).unwrap());
    assert_eq!(report.value.created, vec![Artifact::Plan]);
    assert!(report.value.skipped.is_empty());
}

#[test]
fn create_upgrades_a_light_plan_incrementally() {
    // Q-INCREMENTAL-CREATE: a light start (`plan` only) can be upgraded to
    // full SPDD by naming the missing artifacts, without touching what's
    // already there.
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    let plan_dir = root.join("plans/checkout");

    create(
        &ctx,
        CreateInput {
            feature: "checkout".to_owned(),
            artifacts: vec![Artifact::Plan],
        },
    )
    .unwrap();
    let plan_md_before = fs::read_text(&plan_dir.join("plan.md")).unwrap().unwrap();

    let report = create(
        &ctx,
        CreateInput {
            feature: "checkout".to_owned(),
            artifacts: vec![Artifact::Requirements, Artifact::Analysis],
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert!(fs::is_file(&plan_dir.join("requirements.md")).unwrap());
    assert!(fs::is_file(&plan_dir.join("analysis.md")).unwrap());
    assert_eq!(
        report.value.created,
        vec![Artifact::Requirements, Artifact::Analysis]
    );
    assert!(report.value.skipped.is_empty());

    // plan.md was not regenerated.
    let plan_md_after = fs::read_text(&plan_dir.join("plan.md")).unwrap().unwrap();
    assert_eq!(plan_md_before, plan_md_after);
}

#[test]
fn create_with_a_subset_already_present_is_rejected() {
    // The subset path still refuses when EVERY requested artifact already
    // exists — there is nothing left to write.
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    create(
        &ctx,
        CreateInput {
            feature: "checkout".to_owned(),
            artifacts: vec![Artifact::Plan],
        },
    )
    .unwrap();

    let failure = create(
        &ctx,
        CreateInput {
            feature: "checkout".to_owned(),
            artifacts: vec![Artifact::Plan],
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.already_exists");
}

#[test]
fn the_human_surface_names_created_and_skipped_artifacts() {
    let outcome = CreateOutcome {
        root: Utf8PathBuf::from("/hall"),
        feature: FeatureName::new("checkout").unwrap(),
        plan_dir: Utf8PathBuf::from("/hall/plans/checkout"),
        created: vec![Artifact::Requirements, Artifact::Analysis],
        skipped: vec![Artifact::Plan],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Created requirements.md, analysis.md for `checkout` in /hall/plans/checkout\n\
         Already present, left untouched: plan.md\n"
    );
}

#[test]
fn scaffolded_plan_contains_wave_structure() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    create(
        &ctx,
        CreateInput {
            feature: "checkout".to_owned(),
            artifacts: vec![Artifact::Plan],
        },
    )
    .unwrap();

    let plan_content = fs::read_text(&root.join("plans/checkout/plan.md"))
        .unwrap()
        .unwrap();

    assert!(plan_content.contains("### Wave"));
    assert!(plan_content.contains("**Budget:**"));
    assert!(plan_content.contains("points"));
    assert!(
        plan_content.contains("Red → Green → Refactor")
            || plan_content.contains("Red -> Green -> Refactor")
    );
}
