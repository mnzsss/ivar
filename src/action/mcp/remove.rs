//! `ivar mcp remove` — drop a declared MCP server from `ivar.json`, harness configs and stored secrets.

use std::io;

use serde::Serialize;

use crate::action::{Ctx, discover_hall, read_manifest};
use crate::domain::mcp::McpServerDef;
use crate::error::{Outcome, Report, WriteHuman};
use crate::store::manifest::Manifest;
use crate::store::mcp_secrets::{Change, McpSecrets};

/// Input arguments for removing an MCP server.
#[derive(Debug, Clone)]
pub struct RemoveInput {
    /// The declared server's name.
    pub name: String,
}

/// Result of removing an MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpRemoved {
    pub name: String,
    /// The env var name whose stored value was deleted — never the value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_removed: Option<String>,
    pub providers: Vec<String>,
}

impl WriteHuman for McpRemoved {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Removed MCP server `{}` from: {}.",
            self.name,
            self.providers.join(", ")
        )?;
        if let Some(var) = &self.secret_removed {
            writeln!(w, "Deleted its client secret `{var}`.")?;
        }
        writeln!(
            w,
            "Tokens the providers stored for this server (in their own credential stores) were left there."
        )
    }
}

/// Remove a declared MCP server and materialise harness configs without it.
pub fn remove(ctx: &Ctx, input: &RemoveInput) -> Outcome<McpRemoved> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let removed = super::resolve_server(&manifest, &input.name)?;
    let secrets = McpSecrets::read(&layout)?;
    let servers: Vec<McpServerDef> = manifest
        .mcp_servers()
        .iter()
        .filter(|s| s.name != input.name)
        .cloned()
        .collect();
    let orphan = super::orphan_secret(removed, &servers, &secrets);
    let updated = manifest.with_mcp_servers(servers)?;
    Manifest::write(&layout, &updated)?;
    let providers = super::materialise_all(&layout, &updated)?;
    let secret_removed = match orphan {
        Some(var) => (secrets.remove_and_write(&layout, &var)? == Change::Updated).then_some(var),
        None => None,
    };
    Ok(Report::new(McpRemoved {
        name: input.name.clone(),
        secret_removed,
        providers,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/mcp/remove.rs"]
mod tests;
