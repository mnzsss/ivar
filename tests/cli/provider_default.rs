#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::common::{hall_root, ivar};

#[test]
fn provider_default_reports_json_and_writes_the_manifest() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    ivar()
        .current_dir(&root)
        .args(["provider", "add", "opencode"])
        .assert()
        .success();

    let output = ivar()
        .current_dir(&root)
        .args(["--json", "provider", "default", "opencode"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(value["default"], "opencode");
    assert_eq!(value["previous"], "claude-code");
    assert_eq!(value["changed"], true);
    let manifest = std::fs::read_to_string(root.join("ivar.json")).unwrap();
    assert!(manifest.contains("\"default\": \"opencode\""), "{manifest}");
}

#[test]
fn an_unregistered_provider_exits_two() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    let output = ivar()
        .current_dir(&root)
        .args(["--json", "provider", "default", "omp"])
        .assert()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(value["code"], "provider.not_available");
}
