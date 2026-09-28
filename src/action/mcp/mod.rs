//! `ivar mcp` — authenticate one of the hall's declared MCP servers.
//!
//! One verb today: [`auth`]. See its module doc comment for the three steps
//! and why a registration is never reported as an authentication.

pub mod auth;
pub mod status;

use crate::domain::mcp::McpServerDef;
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::infra::proc::Command;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::store::mcp_secrets::McpSecrets;

pub(crate) fn resolve_server<'a>(
    manifest: &'a Manifest,
    name: &str,
) -> Result<&'a McpServerDef, Failure> {
    manifest
        .mcp_servers()
        .iter()
        .find(|server| server.name == name)
        .ok_or_else(|| {
            let declared: Vec<&str> = manifest
                .mcp_servers()
                .iter()
                .map(|server| server.name.as_str())
                .collect();
            let known = if declared.is_empty() {
                "(no servers declared in ivar.json's `mcp` array)".to_owned()
            } else {
                declared.join(", ")
            };
            Failure::blocked(
                "mcp.server_not_found",
                format!("no MCP server named `{name}` in ivar.json"),
            )
            .expected(format!("one of the declared servers: {known}"))
            .actual(format!("`{name}` is not declared"))
            .fix(FixAction::safe(
                "mcp.check_declared_servers",
                "Check the `mcp` array in ivar.json for the server's declared name.",
            ))
        })
}

pub(crate) fn resolve_provider(
    manifest: &Manifest,
    raw: Option<&str>,
) -> Result<Provider, Failure> {
    match raw {
        Some(value) => value.parse::<Provider>().map_err(Failure::from),
        None => Ok(manifest.providers().default_provider()),
    }
}

/// Inject referenced MCP OAuth client secrets into an OpenCode session command.
///
/// Returns the command unchanged for other providers (e.g. Claude Code).
/// For OpenCode, inspects `manifest.mcp` servers carrying OAuth registrations,
/// resolves each variable name first from the caller environment and then from
/// `.ivar/secrets/mcp.env`, and injects any found values into the child command's environment.
pub fn inject_session_mcp_secrets(
    mut command: Command,
    layout: &Layout,
    manifest: &Manifest,
    provider: Provider,
) -> Command {
    if provider != Provider::OpenCode {
        return command;
    }

    let secrets = McpSecrets::read(layout).ok();

    for server in manifest.mcp_servers() {
        let Some(oauth) = &server.oauth else {
            continue;
        };
        let Some(var) = &oauth.client_secret_env else {
            continue;
        };
        if let Ok(val) = std::env::var(var) {
            command = command.env(var.clone(), val);
        } else if let Some(stored_val) = secrets.as_ref().and_then(|s| s.get(var)) {
            command = command.env(var.clone(), stored_val.to_owned());
        }
    }

    command
}
