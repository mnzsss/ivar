#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::common::{hall_root, ivar};

fn json(root: &camino::Utf8Path, args: &[&str], code: i32) -> serde_json::Value {
    let output = ivar()
        .current_dir(root)
        .arg("--json")
        .args(args)
        .assert()
        .code(code)
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).expect("valid json")
}

#[test]
fn edit_requires_a_url_or_a_command() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    ivar()
        .current_dir(&root)
        .args(["mcp", "edit", "x"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn edit_reports_the_new_definition_as_json() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    json(
        &root,
        &["mcp", "add", "x", "--url", "https://old.test/mcp"],
        0,
    );

    let value = json(
        &root,
        &["mcp", "edit", "x", "--url", "https://new.test/mcp"],
        0,
    );

    assert_eq!(value["url"], "https://new.test/mcp");
    assert_eq!(value["changed"], true);
    let manifest = std::fs::read_to_string(root.join("ivar.json")).unwrap();
    assert!(manifest.contains("https://new.test/mcp"), "{manifest}");
}

#[test]
fn editing_an_unknown_server_exits_two() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    let value = json(
        &root,
        &["mcp", "edit", "ghost", "--url", "https://x.test/mcp"],
        2,
    );

    assert_eq!(value["code"], "mcp.server_not_found");
}

#[test]
fn remove_reports_json_and_clears_the_manifest() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    json(&root, &["mcp", "add", "files", "--command", "npx"], 0);

    let value = json(&root, &["mcp", "remove", "files"], 0);

    assert_eq!(value["name"], "files");
    let manifest = std::fs::read_to_string(root.join("ivar.json")).unwrap();
    assert!(!manifest.contains("\"files\""), "{manifest}");
}

#[test]
fn removing_an_unknown_server_exits_two() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    let value = json(&root, &["mcp", "remove", "ghost"], 2);

    assert_eq!(value["code"], "mcp.server_not_found");
}
