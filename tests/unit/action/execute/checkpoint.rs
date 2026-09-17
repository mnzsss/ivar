#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::discover_hall;
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::domain::feature::{CheckpointKind, RunBaseline, RunId, RunStatus, WaveProgress};
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
fn execute_checkpoint_appends_a_wave_to_the_active_receipt() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();
    let feature = FeatureName::new("child-feature").unwrap();
    RunReceipt::start(
        RunId::new("00000000-0000-4000-8000-000000000001").unwrap(),
        feature.clone(),
        "plans/child-feature/plan.md",
        "hash123",
        RunBaseline::empty(),
        SessionId::new("00000000-0000-4000-8000-000000000002").unwrap(),
        Provider::ClaudeCode,
        rfc3339_now(),
    )
    .write(&layout)
    .unwrap();

    let outcome = checkpoint(
        &ctx,
        CheckpointInput {
            feature: feature.to_string(),
            wave: 1,
            summary: "wave 1 approved".to_owned(),
        },
    )
    .expect("checkpoint should succeed for an active run");

    let stored = RunReceipt::read(&layout, &feature).unwrap().unwrap();
    assert_eq!(stored, outcome.value.receipt);
    assert_eq!(stored.status, RunStatus::Active);
    assert_eq!(stored.plan_fingerprint, "hash123");
    let last = stored.checkpoints.last().unwrap();
    assert_eq!(last.kind, CheckpointKind::Wave);
    assert_eq!(
        last.wave,
        Some(WaveProgress {
            number: 1,
            summary: "wave 1 approved".to_owned(),
        })
    );
}

#[test]
fn execute_checkpoint_refuses_when_no_run_exists() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let failure = checkpoint(
        &ctx,
        CheckpointInput {
            feature: "child-feature".to_owned(),
            wave: 1,
            summary: "nothing to record".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "execute.run_missing");
}
