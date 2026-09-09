#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::domain::feature::{
    CheckStatus, CoordinatorReport, RunBaseline, RunId, RunOutcome, RunProvenance, RunStatus,
    TaskResult, TaskStatus, VerificationCheck,
};
use crate::domain::name::SessionId;
use crate::domain::provider::Provider;
use crate::infra::{fs, hash};
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

fn valid_report() -> CoordinatorReport {
    CoordinatorReport {
        summary: "done".to_owned(),
        tasks: vec![TaskResult {
            title: "t1".to_owned(),
            status: TaskStatus::Completed,
            result: "ok".to_owned(),
        }],
        verification: vec![VerificationCheck {
            command: "cargo test".to_owned(),
            status: CheckStatus::Passed,
            summary: "passed".to_owned(),
        }],
        agents: Vec::new(),
        deviations: Vec::new(),
        blockers: Vec::new(),
        follow_ups: Vec::new(),
    }
}

#[test]
fn execute_finish_succeeds_without_live_session_view_dir() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();
    let feature = FeatureName::new("child-feature").unwrap();

    let plan_path = layout.plan_dir(&feature).join("plan.md");
    fs::write_text(&plan_path, "# Plan\nContent\n").unwrap();
    let plan_fingerprint = hash::file(&plan_path).unwrap();

    let run_id = RunId::new("00000000-0000-4000-8000-000000000001").unwrap();
    let sess_id = SessionId::new("00000000-0000-4000-8000-000000000002").unwrap();
    let receipt = RunReceipt::start(
        run_id.clone(),
        feature.clone(),
        plan_path.clone(),
        plan_fingerprint,
        RunBaseline::empty(),
        sess_id,
        Provider::ClaudeCode,
        rfc3339_now(),
    );
    receipt.write(&layout).unwrap();

    let report_path = layout.feature_dir(&feature).join("report.json");
    fs::write_text(
        &report_path,
        &serde_json::to_string(&valid_report()).unwrap(),
    )
    .unwrap();

    // No live session view dir created under layout.sessions_dir() or feature.sessions_dir()
    let outcome = finish(
        &ctx,
        FinishInput {
            feature: feature.to_string(),
            plan: plan_path.to_string(),
            report_json: report_path.to_string(),
            outcome: "succeeded".to_string(),
        },
    )
    .expect("finish should fallback to receipt coordinator when no live session exists");

    assert_eq!(outcome.value.receipt.status, RunStatus::Succeeded);
    assert_eq!(outcome.value.receipt.outcome, Some(RunOutcome::Succeeded));
    assert!(RunReceipt::read(&layout, &feature).unwrap().is_none());

    let history = run::history(&layout, &feature).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, run_id);
    assert_eq!(history[0].status, RunStatus::Succeeded);
    assert_eq!(history[0].provenance, RunProvenance::Native);
}

#[test]
fn execute_finish_tolerates_checked_boxes_without_plan_diverged() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();
    let feature = FeatureName::new("child-feature").unwrap();

    let plan_path = layout.plan_dir(&feature).join("plan.md");
    let initial_plan = "# Plan\n\n### Wave 1\n- [ ] Task 1\n  - [ ] Subtask 1\n* [ ] Star task 2\n| [ ] | Table Task |\n";
    fs::write_text(&plan_path, initial_plan).unwrap();
    let plan_fingerprint = crate::action::execute::plan_fingerprint::normalized_plan_fingerprint(&plan_path).unwrap();

    let run_id = RunId::new("00000000-0000-4000-8000-000000000001").unwrap();
    let sess_id = SessionId::new("00000000-0000-4000-8000-000000000002").unwrap();
    let receipt = RunReceipt::start(
        run_id.clone(),
        feature.clone(),
        plan_path.clone(),
        plan_fingerprint,
        RunBaseline::empty(),
        sess_id,
        Provider::ClaudeCode,
        rfc3339_now(),
    );
    receipt.write(&layout).unwrap();

    // Toggle checkboxes to checked variants (- [x], - [X], * [x], | [x] |)
    let executed_plan = "# Plan\n\n### Wave 1\n- [x] Task 1\n  - [X] Subtask 1\n* [x] Star task 2\n| [x] | Table Task |\n";
    fs::write_text(&plan_path, executed_plan).unwrap();

    let report_path = layout.feature_dir(&feature).join("report.json");
    fs::write_text(
        &report_path,
        &serde_json::to_string(&valid_report()).unwrap(),
    )
    .unwrap();

    let outcome = finish(
        &ctx,
        FinishInput {
            feature: feature.to_string(),
            plan: plan_path.to_string(),
            report_json: report_path.to_string(),
            outcome: "succeeded".to_string(),
        },
    )
    .expect("finish should succeed when only checkbox states changed");

    assert_eq!(outcome.value.receipt.status, RunStatus::Succeeded);
    assert_eq!(outcome.value.receipt.outcome, Some(RunOutcome::Succeeded));
}

#[test]
fn execute_finish_detects_semantic_plan_divergence() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let layout = discover_hall(&ctx).unwrap();
    let feature = FeatureName::new("child-feature").unwrap();

    let plan_path = layout.plan_dir(&feature).join("plan.md");
    let initial_plan = "# Plan\n\n- [ ] Original Task Description\n";
    fs::write_text(&plan_path, initial_plan).unwrap();
    let plan_fingerprint = crate::action::execute::plan_fingerprint::normalized_plan_fingerprint(&plan_path).unwrap();

    let run_id = RunId::new("00000000-0000-4000-8000-000000000001").unwrap();
    let sess_id = SessionId::new("00000000-0000-4000-8000-000000000002").unwrap();
    let receipt = RunReceipt::start(
        run_id.clone(),
        feature.clone(),
        plan_path.clone(),
        plan_fingerprint,
        RunBaseline::empty(),
        sess_id,
        Provider::ClaudeCode,
        rfc3339_now(),
    );
    receipt.write(&layout).unwrap();

    // Modify task description text
    let diverged_plan = "# Plan\n\n- [x] Modified Task Description\n";
    fs::write_text(&plan_path, diverged_plan).unwrap();

    let report_path = layout.feature_dir(&feature).join("report.json");
    fs::write_text(
        &report_path,
        &serde_json::to_string(&valid_report()).unwrap(),
    )
    .unwrap();

    let err = finish(
        &ctx,
        FinishInput {
            feature: feature.to_string(),
            plan: plan_path.to_string(),
            report_json: report_path.to_string(),
            outcome: "succeeded".to_string(),
        },
    )
    .expect_err("finish should fail when plan text changed");

    assert_eq!(err.code, "execute.plan_diverged");
}
