use camino::Utf8PathBuf;
use serde_json::{Value, json};

use crate::support::common;
use crate::support::graph::TestHall;
use ivar::store::layout::Layout;
use ivar::store::manifest::Manifest;

fn indexed_hall() -> TestHall {
    let hall = TestHall::new();
    hall.commit_base("core", &[("src/lib.rs", "pub fn core_fn() {}\n")]);
    hall
}

fn manifest_path(hall: &TestHall) -> Utf8PathBuf {
    hall.root().join("ivar.json")
}

fn read_manifest_text(hall: &TestHall) -> String {
    std::fs::read_to_string(manifest_path(hall)).unwrap()
}

fn declare_mcp(hall: &TestHall, servers: Value) {
    let layout = Layout::at(hall.root());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let declared = manifest
        .with_mcp_servers(serde_json::from_value(servers).unwrap())
        .unwrap();
    Manifest::write(&layout, &declared).unwrap();
}

fn declared_mcp(hall: &TestHall) -> Value {
    let manifest: Value = serde_json::from_str(&read_manifest_text(hall)).unwrap();
    manifest.get("mcp").cloned().unwrap_or(Value::Null)
}

fn graph_server() -> Value {
    json!({"name": "graph", "type": "local", "command": "ivar", "args": ["graph", "mcp"]})
}

#[test]
fn indexing_registers_the_graph_mcp_server() {
    let hall = indexed_hall();

    let outcome = hall.graph_command(hall.root(), &["index", "--json"]);

    assert_eq!(outcome["mcp_registration"], "registered", "{outcome}");
    assert_eq!(outcome["next_command"], "ivar sync", "{outcome}");
    assert_eq!(declared_mcp(&hall), json!([graph_server()]));
}

#[test]
fn indexing_keeps_ivar_json_canonical() {
    let hall = indexed_hall();
    hall.graph_command(hall.root(), &["index", "--json"]);
    let registered = read_manifest_text(&hall);

    let layout = Layout::at(hall.root());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    assert_eq!(read_manifest_text(&hall), registered);
}

#[test]
fn a_second_index_changes_nothing() {
    let hall = indexed_hall();
    hall.graph_command(hall.root(), &["index", "--json"]);
    let first = read_manifest_text(&hall);

    let outcome = hall.graph_command(hall.root(), &["index", "--json"]);

    assert_eq!(outcome["mcp_registration"], "already_declared", "{outcome}");
    assert!(outcome.get("next_command").is_none(), "{outcome}");
    assert_eq!(read_manifest_text(&hall), first);
}

#[test]
fn an_equivalent_server_under_another_name_counts_as_registered() {
    let hall = indexed_hall();
    declare_mcp(
        &hall,
        json!([{"name": "codebase", "type": "local", "command": "/usr/bin/ivar", "args": ["graph", "mcp", "--tools", "all"]}]),
    );
    let before = read_manifest_text(&hall);

    let outcome = hall.graph_command(hall.root(), &["index", "--json"]);

    assert_eq!(outcome["mcp_registration"], "already_declared", "{outcome}");
    assert_eq!(read_manifest_text(&hall), before);
}

#[test]
fn a_name_clash_warns_and_leaves_ivar_json_untouched() {
    let hall = indexed_hall();
    declare_mcp(
        &hall,
        json!([{"name": "graph", "type": "local", "command": "graphd"}]),
    );
    let before = read_manifest_text(&hall);

    let output = common::ivar()
        .current_dir(hall.root())
        .args(["graph", "index", "--json"])
        .output()
        .unwrap();
    let outcome: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(outcome["mcp_registration"], "name_clash", "{outcome}");
    assert_eq!(outcome["warnings"][0]["code"], "graph.mcp_name_clash");
    assert_eq!(read_manifest_text(&hall), before);
}

#[test]
fn no_mcp_leaves_ivar_json_untouched() {
    let hall = indexed_hall();
    let before = read_manifest_text(&hall);

    let outcome = hall.graph_command(hall.root(), &["index", "--no-mcp", "--json"]);

    assert_eq!(outcome["mcp_registration"], "skipped", "{outcome}");
    assert_eq!(read_manifest_text(&hall), before);
}

#[test]
fn a_manifest_that_cannot_be_written_warns_without_failing_the_index() {
    let hall = TestHall::new();
    common::declare_repos(hall.root(), &[]);
    let before = read_manifest_text(&hall);

    let output = common::ivar()
        .current_dir(hall.root())
        .args(["graph", "index", "--json"])
        .output()
        .unwrap();
    let outcome: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(outcome["mcp_registration"], "failed", "{outcome}");
    assert!(outcome["repos"].is_array(), "{outcome}");
    assert_eq!(
        outcome["warnings"][0]["code"],
        "graph.mcp_registration_failed"
    );
    assert_eq!(read_manifest_text(&hall), before);
}
