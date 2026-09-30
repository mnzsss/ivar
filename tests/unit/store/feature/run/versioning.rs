//! Unit tests for `crate::store::feature::run` — the run receipt's schema
//! version and its v1 → v2 migration.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8Path;
use serde_json::json;

use super::*;
use crate::domain::feature::{Feature, RunBaseline, RunMode};
use crate::domain::name::{BranchName, SessionId};
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::test_support::hall_root;

fn setup_feature(name: &str) -> (tempfile::TempDir, Layout, FeatureName) {
    let (guard, root) = hall_root();
    let layout = Layout::at(&root);
    let feature_name = FeatureName::new(name).unwrap();
    Feature::new(feature_name.clone(), BranchName::new(name).unwrap())
        .write(&layout)
        .unwrap();
    (guard, layout, feature_name)
}

/// A receipt serialized as schema v1 wrote it: `version: 1`, no `mode`.
fn v1_json(id: &str, feature: &FeatureName) -> serde_json::Value {
    let receipt = RunReceipt::start(
        RunId::new(id).unwrap(),
        feature.clone(),
        "plan.md",
        "plan-fp-1",
        RunBaseline::empty(),
        SessionId::new("00000000-0000-4000-8000-0000000000aa").unwrap(),
        Provider::ClaudeCode,
        "2026-08-14T00:00:00Z",
    );
    let mut value = serde_json::to_value(&receipt).unwrap();
    let root = value.as_object_mut().unwrap();
    root.insert("version".to_owned(), json!(1));
    root.remove("mode");
    value
}

fn write_raw(path: &Utf8Path, value: &serde_json::Value) {
    fs::ensure_dir(path.parent().unwrap()).unwrap();
    fs::write_text(path, &serde_json::to_string(value).unwrap()).unwrap();
}

fn on_disk(path: &Utf8Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_text(path).unwrap().unwrap()).unwrap()
}

#[test]
fn a_v1_current_receipt_reads_as_default_mode_and_is_rewritten_as_v2() {
    let (_guard, layout, feature) = setup_feature("feat-v1-current");
    let path = layout.run_receipt(&feature);
    write_raw(
        &path,
        &v1_json("00000000-0000-4000-8000-000000000001", &feature),
    );

    let loaded = RunReceipt::read(&layout, &feature).unwrap().unwrap();

    assert_eq!(loaded.version, 2);
    assert_eq!(loaded.mode, RunMode::Default);
    let persisted = on_disk(&path);
    assert_eq!(persisted["version"], 2);
    assert_eq!(persisted["mode"], "default");
}

#[test]
fn a_v1_archived_receipt_reads_as_default_mode_and_is_rewritten_as_v2() {
    let (_guard, layout, feature) = setup_feature("feat-v1-archived");
    let id = RunId::new("00000000-0000-4000-8000-000000000002").unwrap();
    let path = layout.archived_run(&feature, &id);
    write_raw(
        &path,
        &v1_json("00000000-0000-4000-8000-000000000002", &feature),
    );

    let loaded = RunReceipt::read_archived(&layout, &feature, &id)
        .unwrap()
        .unwrap();

    assert_eq!(loaded.version, 2);
    assert_eq!(loaded.mode, RunMode::Default);
    let persisted = on_disk(&path);
    assert_eq!(persisted["version"], 2);
    assert_eq!(persisted["mode"], "default");
}

#[test]
fn a_receipt_newer_than_v2_is_refused() {
    let (_guard, layout, feature) = setup_feature("feat-v3");
    let path = layout.run_receipt(&feature);
    let mut value = v1_json("00000000-0000-4000-8000-000000000003", &feature);
    value["version"] = json!(3);
    value["mode"] = json!("default");
    write_raw(&path, &value);

    let failure = RunReceipt::read(&layout, &feature).unwrap_err();

    assert_eq!(failure.code, "store.version_too_new");
    assert_eq!(failure.status, Status::Blocked);
}
