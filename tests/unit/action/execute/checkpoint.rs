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
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
            git: false,
            origin: None,
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
    assert_eq!(
        failure.fix_actions[0].command.as_deref(),
        Some("ivar feature execute start child-feature")
    );
}

#[test]
fn execute_checkpoint_refuses_a_run_that_is_over() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();
    let feature = FeatureName::new("child-feature").unwrap();
    let mut receipt = RunReceipt::start(
        RunId::new("00000000-0000-4000-8000-000000000001").unwrap(),
        feature.clone(),
        "plans/child-feature/plan.md",
        "hash123",
        RunBaseline::empty(),
        SessionId::new("00000000-0000-4000-8000-000000000002").unwrap(),
        Provider::ClaudeCode,
        rfc3339_now(),
    );
    receipt.interrupt(rfc3339_now()).unwrap();
    receipt.write(&layout).unwrap();

    let failure = checkpoint(
        &ctx,
        CheckpointInput {
            feature: feature.to_string(),
            wave: 1,
            summary: "too late".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "execute.run_terminal");
    let stored = RunReceipt::read(&layout, &feature).unwrap().unwrap();
    assert_eq!(stored.status, RunStatus::Interrupted);
}

fn start_run_with_plan(root: &Utf8PathBuf, plan: &str) -> (Ctx, FeatureName) {
    let ctx = Ctx::new(root.clone());
    let layout = discover_hall(&ctx).unwrap();
    let feature = FeatureName::new("child-feature").unwrap();
    let plan_path = root.join("plan.md");
    std::fs::write(&plan_path, plan).unwrap();
    RunReceipt::start(
        RunId::new("00000000-0000-4000-8000-000000000001").unwrap(),
        feature.clone(),
        plan_path,
        "hash123",
        RunBaseline::empty(),
        SessionId::new("00000000-0000-4000-8000-000000000002").unwrap(),
        Provider::ClaudeCode,
        rfc3339_now(),
    )
    .write(&layout)
    .unwrap();
    (ctx, feature)
}

fn record(ctx: &Ctx, feature: &FeatureName, wave: u32) -> Outcome<CheckpointOutcome> {
    checkpoint(
        ctx,
        CheckpointInput {
            feature: feature.to_string(),
            wave,
            summary: format!("wave {wave} approved"),
        },
    )
}

#[test]
fn execute_checkpoint_records_wave_zero_without_shifting_it() {
    let (_guard, root) = seeded_hall();
    let (ctx, feature) = start_run_with_plan(
        &root,
        "## Changes\n\n### Wave 0 — split\n\n### Wave 1 — behaviour ✅\n",
    );

    let outcome = record(&ctx, &feature, 0).expect("wave 0 is in the plan");
    record(&ctx, &feature, 1).expect("wave 1 is in the plan");

    let numbers: Vec<u32> = outcome
        .value
        .receipt
        .checkpoints
        .iter()
        .filter_map(|c| c.wave.as_ref().map(|w| w.number))
        .collect();
    assert_eq!(numbers, [0]);
    let stored = RunReceipt::read(&discover_hall(&ctx).unwrap(), &feature)
        .unwrap()
        .unwrap();
    let numbers: Vec<u32> = stored
        .checkpoints
        .iter()
        .filter_map(|c| c.wave.as_ref().map(|w| w.number))
        .collect();
    assert_eq!(numbers, [0, 1]);
}

#[test]
fn execute_checkpoint_keeps_plans_starting_at_wave_one_working() {
    let (_guard, root) = seeded_hall();
    let (ctx, feature) = start_run_with_plan(&root, "### Wave 1 — first\n\n### Wave 2 — second\n");

    record(&ctx, &feature, 1).expect("wave 1 is in the plan");
    let failure = record(&ctx, &feature, 0).unwrap_err();
    assert_eq!(failure.code, "execute.wave_not_in_plan");
}

#[test]
fn execute_checkpoint_names_the_plan_waves_when_the_wave_is_unknown() {
    let (_guard, root) = seeded_hall();
    let (ctx, feature) = start_run_with_plan(
        &root,
        "### Wave 0 — split\n\n```md\n### Wave 9 — example\n```\n\n### Wave 1 — next\n",
    );

    let failure = record(&ctx, &feature, 7).unwrap_err();

    assert_eq!(failure.code, "execute.wave_not_in_plan");
    assert!(
        failure.what.contains("Wave 0, Wave 1") && !failure.what.contains("Wave 9"),
        "{}",
        failure.what
    );
    let stored = RunReceipt::read(&discover_hall(&ctx).unwrap(), &feature)
        .unwrap()
        .unwrap();
    assert!(stored.checkpoints.iter().all(|c| c.wave.is_none()));
}
