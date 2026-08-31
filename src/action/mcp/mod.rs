//! `ivar mcp` — authenticate one of the hall's declared MCP servers.
//!
//! One verb today: [`auth`]. See its module doc comment for the three steps
//! and why a registration is never reported as an authentication.

pub mod auth;

use crate::domain::provider::Provider;
use crate::infra::proc::Command;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::store::mcp_secrets::McpSecrets;

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
        let var = &oauth.client_secret_env;
        if let Ok(val) = std::env::var(var) {
            command = command.env(var.clone(), val);
        } else if let Some(stored_val) = secrets.as_ref().and_then(|s| s.get(var)) {
            command = command.env(var.clone(), stored_val.to_owned());
        }
    }

    command
}
