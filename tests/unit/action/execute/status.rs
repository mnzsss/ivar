//! Unit tests for `crate::action::execute::status`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(clippy::unwrap_used)]

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::domain::feature::{RunBaseline, RunStatus};
use crate::domain::name::SessionId;
use crate::domain::provider::Provider;
use crate::test_support::hall_root;
use camino::Utf8PathBuf;

#[test]
fn human_output_includes_receipt_recovery_plan_evidence_and_provenance() {
    let mut receipt = RunReceipt::start(
        RunId::new("00000000-0000-0000-0000-000000000001").unwrap(),
        FeatureName::new("checkout").unwrap(),
        "plans/checkout/plan.md",
        "plan-fingerprint",
        RunBaseline::empty(),
        SessionId::new("00000000-0000-0000-0000-000000000002").unwrap(),
        Provider::ClaudeCode,
        "2026-01-01T00:00:00Z",
    );
    receipt.status = RunStatus::Blocked;
    let mut output = Vec::new();

    StatusOutcome {
        receipts: vec![receipt],
    }
    .write_human(&mut output)
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("plan: plans/checkout/plan.md (plan-fingerprint)"));
    assert!(output.contains("provenance: native"));
    assert!(output.contains("recovery: resume with `execute start --resume`"));
    assert!(output.contains("evidence: no final filesystem evidence"));
}
#[test]
fn human_output_includes_mode() {
    let receipt = RunReceipt::start(
        RunId::new("00000000-0000-0000-0000-000000000001").unwrap(),
        FeatureName::new("checkout").unwrap(),
        "plans/checkout/plan.md",
        "plan-fingerprint",
        RunBaseline::empty(),
        SessionId::new("00000000-0000-0000-0000-000000000002").unwrap(),
        Provider::ClaudeCode,
        "2026-01-01T00:00:00Z",
    )
    .with_mode(crate::domain::feature::RunMode::Goal);

    let mut output = Vec::new();
    StatusOutcome {
        receipts: vec![receipt],
    }
    .write_human(&mut output)
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("  mode: goal"));
}

fn seeded_hall() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        &InitInput {
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

fn default_status(feature: &FeatureName) -> StatusInput {
    StatusInput {
        feature: feature.to_string(),
        plan: None,
        history: false,
        run: None,
    }
}

#[test]
fn status_without_flags_shows_the_finished_run() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();
    let feature = FeatureName::new("child-feature").unwrap();
    let run_id = RunId::new("00000000-0000-4000-8000-000000000001").unwrap();
    let mut receipt = RunReceipt::start(
        run_id.clone(),
        feature.clone(),
        "plans/child-feature/plan.md",
        "hash123",
        RunBaseline::empty(),
        SessionId::new("00000000-0000-4000-8000-000000000002").unwrap(),
        Provider::ClaudeCode,
        "2026-01-01T00:00:00Z",
    );
    receipt.status = RunStatus::Succeeded;
    receipt.write(&layout).unwrap();
    run::archive_current(&layout, &feature).unwrap();

    let outcome = status(&ctx, default_status(&feature)).unwrap();

    assert_eq!(outcome.value.receipts.len(), 1);
    let receipt = outcome.value.receipts.first().unwrap();
    assert_eq!(receipt.id, run_id);
    assert_eq!(receipt.status, RunStatus::Succeeded);
}

#[test]
fn status_without_any_run_is_run_missing() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let feature = FeatureName::new("child-feature").unwrap();

    let failure = status(&ctx, default_status(&feature)).unwrap_err();

    assert_eq!(failure.code, "execute.run_missing");
}
