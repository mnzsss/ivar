#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use super::*;
use crate::action::execute::finish::{FinishInput, finish};
use crate::action::execute::start as execute_start;
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::approve::{self, ApproveInput};
use crate::domain::feature::RunStatus;
use crate::domain::name::SessionId;
use crate::domain::provider::Provider;
use crate::domain::session::SessionState;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::test_support::hall_root;

fn seeded_execution_hall() -> (tempfile::TempDir, Utf8PathBuf) {
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

fn approve_plan(ctx: &Ctx) {
    approve::approve(
        ctx,
        ApproveInput {
            feature: "child-feature".to_owned(),
            gate: "plan".to_owned(),
        },
    )
    .unwrap();
}

fn write_feature_session(layout: &Layout, feature: &FeatureName) {
    let sess_id = SessionId::new("00000000-0000-4000-8000-000000000002").unwrap();
    let view_dir = layout.feature_session(feature, &sess_id);
    fs::ensure_dir(&view_dir).unwrap();
    let mut state = SessionState::new(Provider::ClaudeCode, "2026-01-01T00:00:00Z");
    state.bind(feature.clone(), "2026-01-01T00:00:00Z");
    state.write(&view_dir).unwrap();
}

fn write_report(layout: &Layout, feature: &FeatureName) -> Utf8PathBuf {
    let report_path = layout.feature_dir(feature).join("report.json");
    let report = crate::domain::feature::CoordinatorReport {
        summary: "done".to_owned(),
        tasks: vec![crate::domain::feature::TaskResult {
            title: "t1".to_owned(),
            status: crate::domain::feature::TaskStatus::Completed,
            result: "ok".to_owned(),
        }],
        verification: vec![crate::domain::feature::VerificationCheck {
            command: "cargo test".to_owned(),
            status: crate::domain::feature::CheckStatus::Passed,
            summary: "passed".to_owned(),
        }],
        agents: Vec::new(),
        deviations: Vec::new(),
        blockers: Vec::new(),
        follow_ups: Vec::new(),
    };
    fs::write_text(&report_path, &serde_json::to_string(&report).unwrap()).unwrap();
    report_path
}

#[test]
fn execute_start_pins_normalized_plan_fingerprint() {
    let (_guard, root) = seeded_execution_hall();
    let ctx = Ctx::new(root.clone());
    let layout = discover_hall(&ctx).unwrap();
    let feature = FeatureName::new("child-feature").unwrap();
    let plan = layout.plan_dir(&feature).join("plan.md");
    fs::write_text(&plan, "# Plan\n\n- [ ] Execute task\n").unwrap();
    approve_plan(&ctx);
    write_feature_session(&layout, &feature);

    execute_start::start(
        &ctx,
        execute_start::StartInput {
            feature: feature.to_string(),
            plan: plan.to_string(),
            resume: false,
            restart: false,
        },
    )
    .unwrap();
    fs::write_text(&plan, "# Plan\n\n- [x] Execute task\n").unwrap();

    let report = write_report(&layout, &feature);
    let outcome = finish(
        &ctx,
        FinishInput {
            feature: feature.to_string(),
            plan: plan.to_string(),
            report_json: report.to_string(),
            outcome: "succeeded".to_owned(),
        },
    )
    .expect("checkbox progress must not diverge a receipt created by execute start");

    assert_eq!(outcome.value.receipt.status, RunStatus::Succeeded);
}
