#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::domain::mcp::McpServerDef;
use crate::store::layout::Layout;

fn graph_entry() -> McpServerDef {
    McpServerDef::new("graph", "local")
        .command("ivar")
        .args(vec!["graph".to_owned(), "mcp".to_owned()])
}

fn manifest_with(servers: Vec<McpServerDef>) -> Manifest {
    let (_guard, root) = crate::test_support::seeded_hall();
    let base = crate::action::read_manifest(&Layout::at(root)).unwrap();
    base.with_mcp_servers(servers).unwrap()
}

#[test]
fn a_default_graph_entry_is_renamed_to_ivar() {
    let renamed = rename_legacy_graph_server(&manifest_with(vec![graph_entry()]))
        .unwrap()
        .expect("renamed");
    assert_eq!(renamed.mcp_servers().first().unwrap().name, "ivar");
}

#[test]
fn a_manifest_already_on_ivar_is_left_alone() {
    let entry = McpServerDef::new("ivar", "local")
        .command("ivar")
        .args(vec!["graph".to_owned(), "mcp".to_owned()]);
    assert!(
        rename_legacy_graph_server(&manifest_with(vec![entry]))
            .unwrap()
            .is_none()
    );
}

#[test]
fn an_edited_graph_entry_is_not_touched_and_warns() {
    let edited = graph_entry().args(vec![
        "graph".to_owned(),
        "mcp".to_owned(),
        "--tools".to_owned(),
        "all".to_owned(),
    ]);
    let warning = rename_legacy_graph_server(&manifest_with(vec![edited])).unwrap_err();
    assert_eq!(warning.code, "graph.mcp_name_clash");
}

#[test]
fn a_foreign_ivar_server_blocks_the_rename() {
    let foreign = McpServerDef::new("ivar", "local").command("other");
    let warning =
        rename_legacy_graph_server(&manifest_with(vec![graph_entry(), foreign])).unwrap_err();
    assert_eq!(warning.code, "graph.mcp_name_clash");
}

#[test]
fn a_user_owned_graph_server_is_left_alone_without_a_warning() {
    let own = McpServerDef::new("graph", "local").command("my-graph-server");
    assert!(
        rename_legacy_graph_server(&manifest_with(vec![own]))
            .unwrap()
            .is_none()
    );
}
