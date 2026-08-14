#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput};
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
fn list_reports_no_plans_in_a_fresh_hall() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let report = list(&ctx).unwrap();

    assert!(report.is_clean());
    assert!(report.value.plans.is_empty());
}

#[test]
fn list_reports_created_plans_with_their_artifacts() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
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
    plan_create::create(
        &ctx,
        CreateInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let report = list(&ctx).unwrap();

    assert_eq!(report.value.plans.len(), 1);
    let plan = report.value.plans.first().unwrap();
    assert_eq!(plan.feature.as_str(), "checkout");
    assert_eq!(plan.artifacts.len(), 3);
}

#[test]
fn the_human_surface_lists_artifacts_per_feature() {
    let outcome = ListOutcome {
        root: Utf8PathBuf::from("/hall"),
        plans: vec![PlanSummary {
            feature: FeatureName::new("checkout").unwrap(),
            artifacts: vec!["requirements.md".to_owned(), "plan.md".to_owned()],
        }],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Plans in /hall:\n  checkout  [requirements.md, plan.md]\n"
    );
}
