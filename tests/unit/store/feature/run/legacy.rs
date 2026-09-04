#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;

use camino::Utf8PathBuf;
use serde_json::json;

use super::*;
use crate::domain::feature::{
    Feature, LegacyEvidence, RunBaseline, RunCheckpoint, RunOutcome, RunProvenance, RunStatus,
};
use crate::domain::name::{BranchName, FeatureName};
use crate::error::Status;
use crate::infra::fs;
use crate::test_support::hall_root;

fn setup_feature(name: &str) -> (tempfile::TempDir, Layout, FeatureName) {
    let (guard, root) = hall_root();
    let layout = Layout::at(&root);
    let feature_name = FeatureName::new(name).unwrap();
    let feature = Feature::new(feature_name.clone(), BranchName::new(name).unwrap());
    feature.write(&layout).unwrap();
    (guard, layout, feature_name)
}

fn sample_board_v3() -> serde_json::Value {
    json!({
        "version": 3,
        "status": "completed",
        "graph": {
            "workstreams": [
                {
                    "id": "ws-1",
                    "title": "First workstream",
                    "status": "completed",
                    "operations": ["op-1"],
                    "depends_on": []
                }
            ],
            "plan_fingerprint": "sha256:abc"
        },
        "journal": [
            {
                "seq": 1,
                "timestamp": "2026-09-03T00:00:00Z",
                "workstream": "ws-1",
                "kind": "tick",
                "message": "started",
                "revision": "sha256:abc"
            }
        ],
        "blocked_by": [],
        "sessions": {
            "ws-1": "sess-1"
        },
        "next_event_seq": 2
    })
}

fn test_run_id(id_str: &str) -> RunId {
    RunId::new(id_str).unwrap()
}

#[test]
fn absent_board_imports_as_none() {
    let (_guard, layout, feature) = setup_feature("feat-absent");
    let result = import(
        &layout,
        &feature,
        Utf8PathBuf::from("plan.md"),
        test_run_id("00000000-0000-4000-8000-000000000001"),
        "2026-09-03T00:00:00Z",
    )
    .expect("import succeeds");

    assert_eq!(result, None);
}

#[test]
fn resumed_import_reuses_the_first_runs_id() {
    let (_guard, layout, feature) = setup_feature("feat-resume");
    let board_file = board_path(&layout, &feature);
    fs::ensure_dir(&layout.execution_dir(&feature)).unwrap();
    json::write_canonical(&board_file, &sample_board_v3()).unwrap();

    let id_a = test_run_id("00000000-0000-4000-8000-000000000001");
    let plan_path = Utf8PathBuf::from("plan.md");
    let at = "2026-09-03T00:00:00Z";

    // Simulate crash after Step 2: archive_board and run.json (receipt) written,
    // but run.json not yet archived/removed and board.json still present.
    let raw = json::read::<serde_json::Value>(&board_file)
        .unwrap()
        .unwrap();
    let normalized = normalize(&board_file, raw).unwrap();
    let canonical = json::to_canonical_string(&normalized).unwrap();
    let source_hash = crate::infra::hash::text(&canonical);
    let archived_board = archive_board(&layout, &feature, &source_hash, &canonical).unwrap();
    let evidence = evidence(&normalized, source_hash, archived_board.clone()).unwrap();
    let (status, outcome) = outcome_of(&evidence.board_status);
    let receipt_a = RunReceipt::from_legacy(
        id_a.clone(),
        feature.clone(),
        plan_path.clone(),
        status,
        outcome,
        evidence,
        at,
    );
    receipt_a.write(&layout).unwrap();

    // Now call import with RunId B
    let id_b = test_run_id("00000000-0000-4000-8000-000000000002");
    let result = import(&layout, &feature, plan_path, id_b, at)
        .expect("import succeeds")
        .expect("import result exists");

    assert!(result.resumed, "import should report resumed: true");
    assert_eq!(
        result.receipt.id, id_a,
        "resumed import must keep first run id"
    );
    assert_eq!(result.archived_board, archived_board);

    // After import finishes, board.json should be removed and run.json archived
    assert!(!fs::exists(&board_file).unwrap());
    assert!(
        RunReceipt::read_archived(&layout, &feature, &id_a)
            .unwrap()
            .is_some()
    );
    assert!(RunReceipt::read(&layout, &feature).unwrap().is_none());
}

#[test]
fn a_board_newer_than_this_ivar_is_refused() {
    let (_guard, layout, feature) = setup_feature("feat-too-new");
    let board_file = board_path(&layout, &feature);
    fs::ensure_dir(&layout.execution_dir(&feature)).unwrap();
    let mut board = sample_board_v3();
    board["version"] = json!(BOARD_VERSION + 1);
    json::write_canonical(&board_file, &board).unwrap();

    let failure = import(
        &layout,
        &feature,
        Utf8PathBuf::from("plan.md"),
        test_run_id("00000000-0000-4000-8000-000000000001"),
        "2026-09-03T00:00:00Z",
    )
    .unwrap_err();

    assert_eq!(failure.code, "store.version_too_new");
    assert_eq!(failure.status, Status::Blocked);
}

#[test]
fn a_board_missing_optional_fields_still_imports() {
    let (_guard, layout, feature) = setup_feature("feat-permissive");
    let board_file = board_path(&layout, &feature);
    fs::ensure_dir(&layout.execution_dir(&feature)).unwrap();
    // Minimal board with just version
    let minimal = json!({
        "version": 1
    });
    json::write_canonical(&board_file, &minimal).unwrap();

    let id = test_run_id("00000000-0000-4000-8000-000000000001");
    let result = import(
        &layout,
        &feature,
        Utf8PathBuf::from("plan.md"),
        id.clone(),
        "2026-09-03T00:00:00Z",
    )
    .expect("minimal board imports successfully")
    .expect("import result exists");

    assert!(!result.resumed);
    assert_eq!(result.receipt.id, id);
    assert_eq!(result.receipt.status, RunStatus::Interrupted);
    assert_eq!(result.receipt.outcome, None);
    assert_eq!(result.receipt.legacy.as_ref().unwrap().workstreams.len(), 0);
    assert_eq!(result.receipt.legacy.as_ref().unwrap().journal.len(), 0);
}

#[test]
fn a_board_v3_missing_optional_fields_still_imports() {
    let (_guard, layout, feature) = setup_feature("feat-permissive-v3");
    let board_file = board_path(&layout, &feature);
    fs::ensure_dir(&layout.execution_dir(&feature)).unwrap();
    // Minimal v3 board with only version and status, omitting graph/journal
    let minimal = json!({
        "version": 3,
        "status": "completed"
    });
    json::write_canonical(&board_file, &minimal).unwrap();

    let id = test_run_id("00000000-0000-4000-8000-000000000001");
    let result = import(
        &layout,
        &feature,
        Utf8PathBuf::from("plan.md"),
        id.clone(),
        "2026-09-03T00:00:00Z",
    )
    .expect("minimal v3 board imports successfully")
    .expect("import result exists");

    assert!(!result.resumed);
    assert_eq!(result.receipt.id, id);
    assert_eq!(result.receipt.status, RunStatus::Succeeded);
    assert_eq!(result.receipt.outcome, Some(RunOutcome::Succeeded));
    assert_eq!(result.receipt.legacy.as_ref().unwrap().workstreams.len(), 0);
    assert_eq!(result.receipt.legacy.as_ref().unwrap().journal.len(), 0);
}

#[test]
fn a_board_with_malformed_workstream_is_refused() {
    let (_guard, layout, feature) = setup_feature("feat-malformed-ws");
    let board_file = board_path(&layout, &feature);
    fs::ensure_dir(&layout.execution_dir(&feature)).unwrap();
    let malformed = json!({
        "version": 1,
        "graph": {
            "workstreams": ["not an object"]
        }
    });
    json::write_canonical(&board_file, &malformed).unwrap();

    let failure = import(
        &layout,
        &feature,
        Utf8PathBuf::from("plan.md"),
        test_run_id("00000000-0000-4000-8000-000000000001"),
        "2026-09-03T00:00:00Z",
    )
    .unwrap_err();

    assert_eq!(failure.code, "store.migration_failed");
    assert_eq!(failure.status, Status::Blocked);
    assert!(failure.what.contains("workstream not an object"));
}

#[test]
fn a_board_with_malformed_journal_entry_is_refused() {
    let (_guard, layout, feature) = setup_feature("feat-malformed-journal");
    let board_file = board_path(&layout, &feature);
    fs::ensure_dir(&layout.execution_dir(&feature)).unwrap();
    let malformed = json!({
        "version": 1,
        "journal": ["not an object"]
    });
    json::write_canonical(&board_file, &malformed).unwrap();

    let failure = import(
        &layout,
        &feature,
        Utf8PathBuf::from("plan.md"),
        test_run_id("00000000-0000-4000-8000-000000000001"),
        "2026-09-03T00:00:00Z",
    )
    .unwrap_err();

    assert_eq!(failure.code, "store.migration_failed");
    assert_eq!(failure.status, Status::Blocked);
    assert!(failure.what.contains("journal entry not an object"));
}

#[test]
fn an_import_over_a_different_board_is_refused() {
    let (_guard, layout, feature) = setup_feature("feat-conflict");
    let board_file = board_path(&layout, &feature);
    fs::ensure_dir(&layout.execution_dir(&feature)).unwrap();

    // Write first board and import it partially by writing a legacy receipt with a different source_hash to run.json
    let board_a = json!({
        "version": 3,
        "status": "completed",
        "graph": { "workstreams": [], "plan_fingerprint": "hash-a" }
    });
    json::write_canonical(&board_file, &board_a).unwrap();

    let fake_receipt = RunReceipt::from_legacy(
        test_run_id("00000000-0000-4000-8000-000000000001"),
        feature.clone(),
        Utf8PathBuf::from("plan.md"),
        RunStatus::Succeeded,
        Some(RunOutcome::Succeeded),
        LegacyEvidence {
            source_hash: "different-source-hash".to_owned(),
            board_status: "completed".to_owned(),
            plan_fingerprint: None,
            workstreams: Vec::new(),
            sessions: BTreeMap::new(),
            journal: Vec::new(),
            archived_board: layout.archived_board(&feature, "different-source-hash"),
        },
        "2026-09-03T00:00:00Z",
    );
    fake_receipt.write(&layout).unwrap();

    let failure = import(
        &layout,
        &feature,
        Utf8PathBuf::from("plan.md"),
        test_run_id("00000000-0000-4000-8000-000000000002"),
        "2026-09-03T00:00:00Z",
    )
    .unwrap_err();

    assert_eq!(failure.code, "execute.legacy_source_conflict");
    assert_eq!(failure.status, Status::Blocked);
}

#[test]
fn an_import_over_a_live_native_run_is_refused() {
    let (_guard, layout, feature) = setup_feature("feat-live-run");
    let board_file = board_path(&layout, &feature);
    fs::ensure_dir(&layout.execution_dir(&feature)).unwrap();
    json::write_canonical(&board_file, &sample_board_v3()).unwrap();

    // Write a live native run to run.json (e.g. RunStatus::Active with native provenance)
    let native_receipt = RunReceipt {
        version: crate::domain::feature::RUN_CURRENT_VERSION,
        id: test_run_id("00000000-0000-4000-8000-000000000001"),
        feature: feature.clone(),
        provenance: RunProvenance::Native,
        status: RunStatus::Active,
        plan_path: Utf8PathBuf::from("plan.md"),
        plan_fingerprint: "fp".to_owned(),
        started_at: "2026-09-03T00:00:00Z".to_owned(),
        updated_at: "2026-09-03T00:00:00Z".to_owned(),
        terminated_at: None,
        coordinators: Vec::new(),
        baseline: RunBaseline::empty(),
        checkpoints: vec![RunCheckpoint {
            at: "2026-09-03T00:00:00Z".to_owned(),
            kind: crate::domain::feature::CheckpointKind::Started,
            status: RunStatus::Active,
            session: None,
            provider: None,
            report: None,
            diff: None,
            plan_fingerprint_from: None,
            plan_fingerprint_to: None,
        }],
        final_diff: None,
        outcome: None,
        legacy: None,
    };
    native_receipt.write(&layout).unwrap();

    let failure = import(
        &layout,
        &feature,
        Utf8PathBuf::from("plan.md"),
        test_run_id("00000000-0000-4000-8000-000000000002"),
        "2026-09-03T00:00:00Z",
    )
    .unwrap_err();

    assert_eq!(failure.code, "execute.legacy_import_blocked");
    assert_eq!(failure.status, Status::Blocked);
}
