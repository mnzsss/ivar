#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::domain::feature::{RunBaseline, RunId, RunProvenance, RunStatus};
use crate::domain::name::SessionId;
use crate::domain::provider::Provider;
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
            name: "child-feature".to_owned(),
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
fn execute_interrupt_archives_active_run_receipt() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();
    let feature = FeatureName::new("child-feature").unwrap();
    let run_id = RunId::new("00000000-0000-4000-8000-000000000001").unwrap();
    let sess_id = SessionId::new("00000000-0000-4000-8000-000000000002").unwrap();
    let receipt = RunReceipt::start(
        run_id.clone(),
        feature.clone(),
        "plans/child-feature/plan.md",
        "hash123",
        RunBaseline::empty(),
        sess_id,
        Provider::ClaudeCode,
        rfc3339_now(),
    );
    receipt.write(&layout).unwrap();

    let outcome = interrupt(
        &ctx,
        InterruptInput {
            feature: feature.to_string(),
        },
    )
    .expect("interrupt should succeed for active run");

    assert_eq!(outcome.value.receipt.status, RunStatus::Interrupted);
    assert!(RunReceipt::read(&layout, &feature).unwrap().is_none());

    let history = run::history(&layout, &feature).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, run_id);
    assert_eq!(history[0].status, RunStatus::Interrupted);
    assert_eq!(history[0].provenance, RunProvenance::Native);
}

#[test]
fn execute_interrupt_refuses_when_no_active_run() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let feature = FeatureName::new("child-feature").unwrap();

    let failure = interrupt(
        &ctx,
        InterruptInput {
            feature: feature.to_string(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "execute.run_missing");
}
