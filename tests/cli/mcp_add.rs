//! End-to-end tests for `ivar mcp add`, driving the compiled binary.
//!
//! The duplicate rule and manifest write are unit-tested in the action. These
//! prove the exit codes and that `--json` agrees with the files on disk.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::common::{hall_root, ivar};

#[test]
fn add_figma_writes_the_manifest_and_the_provider_config() {
    let (_guard, root) = hall_root();
    ivar()
        .current_dir(&root)
        .args(["init", "--name", "acme", "--provider", "opencode"])
        .assert()
        .success();

    let output = ivar()
        .current_dir(&root)
        .args(["mcp", "add", "figma", "--json"])
        .assert()
        .success()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(value["name"], "figma");
    assert_eq!(value["url"], "https://mcp.figma.com/mcp");
    assert_eq!(value["changed"], true);
    assert_eq!(value["next"], "ivar mcp auth figma --provider opencode");

    let manifest = std::fs::read_to_string(root.join("ivar.json")).unwrap();
    assert!(
        manifest.contains("\"url\": \"https://mcp.figma.com/mcp\""),
        "{manifest}"
    );
    let opencode = std::fs::read_to_string(root.join("opencode.json")).unwrap();
    assert!(opencode.contains("acme-figma"), "{opencode}");
}

#[test]
fn adding_the_same_server_twice_changes_nothing() {
    let (_guard, root) = hall_root();
    ivar()
        .current_dir(&root)
        .args(["init", "--provider", "opencode"])
        .assert()
        .success();
    ivar()
        .current_dir(&root)
        .args(["mcp", "add", "figma"])
        .assert()
        .success();
    let before = std::fs::read_to_string(root.join("ivar.json")).unwrap();

    let output = ivar()
        .current_dir(&root)
        .args(["mcp", "add", "figma", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(value["changed"], false);
    assert_eq!(
        std::fs::read_to_string(root.join("ivar.json")).unwrap(),
        before
    );
}

#[test]
fn an_unknown_name_without_url_is_blocked_with_exit_two() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    let output = ivar()
        .current_dir(&root)
        .args(["mcp", "add", "nope", "--json"])
        .assert()
        .failure()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(value["kind"], "blocked");
    assert_eq!(value["code"], "mcp.unknown_preset");
}

#[test]
fn url_and_command_together_are_a_usage_error() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    ivar()
        .current_dir(&root)
        .args([
            "mcp",
            "add",
            "x",
            "--url",
            "https://x.test/mcp",
            "--command",
            "npx",
        ])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn add_stdio_reports_the_definition_as_json() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    let output = ivar()
        .current_dir(&root)
        .args([
            "--json",
            "mcp",
            "add",
            "files",
            "--command",
            "npx",
            "--",
            "-y",
            "@mcp/fs",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(value["command"], "npx");
    assert_eq!(value["args"], serde_json::json!(["-y", "@mcp/fs"]));
    assert_eq!(value["changed"], true);
    let manifest = std::fs::read_to_string(root.join("ivar.json")).unwrap();
    assert!(manifest.contains("\"type\": \"local\""), "{manifest}");
}
