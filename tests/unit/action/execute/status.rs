//! Unit tests for `crate::action::execute::status`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(clippy::unwrap_used)]

use super::*;
use crate::domain::feature::{RunBaseline, RunStatus};
use crate::domain::name::SessionId;
use crate::domain::provider::Provider;

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
