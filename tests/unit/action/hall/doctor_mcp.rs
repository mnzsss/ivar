use super::*;
use crate::action::hall::doctor::mcp::mcp_name_diagnoses;

fn graph_entry() -> McpServerDef {
    McpServerDef::new("graph", "local")
        .command("ivar")
        .args(vec!["graph".to_owned(), "mcp".to_owned()])
}

#[test]
fn a_manifest_still_naming_the_server_graph_is_reported() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let manifest = crate::action::read_manifest(&layout)
        .unwrap()
        .with_mcp_servers(vec![graph_entry()])
        .unwrap();

    let codes: Vec<_> = mcp_name_diagnoses(&layout, &manifest)
        .iter()
        .map(|d| d.code)
        .collect();

    assert!(codes.contains(&"mcp.legacy_name"));
}

#[test]
fn a_provider_config_still_keyed_hall_graph_is_reported_with_the_sync_fix() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    std::fs::write(
        root.join(".mcp.json"),
        r#"{"mcpServers":{"acme-graph":{"type":"stdio","command":"ivar","args":["graph","mcp"]}}}"#,
    )
    .unwrap();
    let manifest = crate::action::read_manifest(&layout).unwrap();

    let findings = mcp_name_diagnoses(&layout, &manifest);

    let stale = findings
        .iter()
        .find(|d| d.code == "mcp.provider_config_stale")
        .unwrap();
    assert!(stale.fix.contains("ivar sync"));
}

#[test]
fn an_ivar_named_hall_reports_nothing() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let manifest = crate::action::read_manifest(&layout).unwrap();
    assert!(mcp_name_diagnoses(&layout, &manifest).is_empty());
}
