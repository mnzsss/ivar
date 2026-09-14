use std::path::Path;

use crate::domain::mcp::McpServerDef;
use crate::error::Warning;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, MigrationPlan};

use super::outcome::McpRegistration;

const SERVER_NAME: &str = "graph";

fn graph_server() -> McpServerDef {
    McpServerDef::new(SERVER_NAME, "local")
        .command("ivar")
        .args(vec!["graph".to_owned(), "mcp".to_owned()])
}

fn runs_graph_mcp(server: &McpServerDef) -> bool {
    let is_ivar = server
        .command
        .as_deref()
        .and_then(|command| Path::new(command).file_stem())
        .is_some_and(|stem| stem == "ivar");
    let args = server.args.as_deref().unwrap_or_default();
    is_ivar && args.starts_with(&["graph".to_owned(), "mcp".to_owned()])
}

pub fn register_graph_mcp(
    layout: &Layout,
    manifest: &Manifest,
) -> (McpRegistration, Option<Warning>) {
    let servers = manifest.mcp_servers();
    if servers.iter().any(runs_graph_mcp) {
        return (McpRegistration::AlreadyDeclared, None);
    }
    if servers.iter().any(|server| server.name == SERVER_NAME) {
        let warning = Warning::new(
            "graph.mcp_name_clash",
            "ivar.json",
            "an MCP server named `graph` already exists with a different command, so the graph \
             MCP server was not registered; add `{\"name\":\"ivar-graph\",\"type\":\"local\",\
             \"command\":\"ivar\",\"args\":[\"graph\",\"mcp\"]}` to `mcp` by hand, then run `ivar sync`",
        );
        return (McpRegistration::NameClash, Some(warning));
    }
    if !matches!(
        Manifest::plan(layout),
        Ok(Some(MigrationPlan::Current { .. }))
    ) {
        return (McpRegistration::NeedsMigration, None);
    }
    let mut updated = servers.to_vec();
    updated.push(graph_server());
    let written = manifest
        .with_mcp_servers(updated)
        .and_then(|registered| Manifest::write(layout, &registered));
    match written {
        Ok(()) => (McpRegistration::Registered, None),
        Err(err) => {
            let warning = Warning::new(
                "graph.mcp_registration_failed",
                "ivar.json",
                format!("the graph MCP server was not registered: {err}"),
            );
            (McpRegistration::Failed, Some(warning))
        }
    }
}
