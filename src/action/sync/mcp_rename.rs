use crate::action::graph::mcp_registration::{LEGACY_SERVER_NAME, SERVER_NAME, runs_graph_mcp};
use crate::domain::mcp::McpServerDef;
use crate::error::Warning;
use crate::store::manifest::Manifest;

fn is_default(server: &McpServerDef) -> bool {
    server.type_ == "local"
        && server.env.is_none()
        && server.args.as_deref() == Some(&["graph".to_owned(), "mcp".to_owned()][..])
}

pub(crate) fn rename_legacy_graph_server(manifest: &Manifest) -> Result<Option<Manifest>, Warning> {
    let servers = manifest.mcp_servers();
    let Some(legacy) = servers.iter().find(|s| s.name == LEGACY_SERVER_NAME) else {
        return Ok(None);
    };
    if !runs_graph_mcp(legacy) {
        return Ok(None);
    }
    let taken = servers.iter().any(|s| s.name == SERVER_NAME);
    if !is_default(legacy) || taken {
        return Err(Warning::new(
            "graph.mcp_name_clash",
            "ivar.json",
            "the `graph` MCP server was not renamed to `ivar` because the entry was edited or `ivar` is taken; rename it by hand, then run `ivar sync`",
        ));
    }
    let renamed = servers
        .iter()
        .map(|server| {
            let mut server = server.clone();
            if server.name == LEGACY_SERVER_NAME {
                SERVER_NAME.clone_into(&mut server.name);
            }
            server
        })
        .collect();
    manifest
        .with_mcp_servers(renamed)
        .map(Some)
        .map_err(|error| {
            Warning::new(
                "graph.mcp_registration_failed",
                "ivar.json",
                error.to_string(),
            )
        })
}

#[cfg(test)]
#[path = "../../../tests/unit/action/sync/mcp_rename.rs"]
mod tests;
