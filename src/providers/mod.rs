//! Closed facade dispatching provider-native behaviors by `Provider`.

pub mod claude_code;
pub mod omp;
pub mod opencode;

use crate::domain::guard::{GuardDecision, GuardOutcome, ToolRequest};
use crate::domain::mcp::McpServerDef;
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::infra::proc::Command;
use camino::Utf8PathBuf;

/// A managed standalone file artifact owned by a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedArtifact {
    pub relative_path: Utf8PathBuf,
    pub contents: &'static str,
}

/// What a provider harness can and cannot do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub supports_resume: bool,
    pub supports_review: bool,
    pub interactive: bool,
}

/// The launch specification for a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchContract {
    pub binary: &'static str,
    pub capabilities: Capabilities,
}

/// Returns the launch contract (binary and capabilities) for a provider.
#[must_use]
pub fn launch_contract(provider: Provider) -> LaunchContract {
    match provider {
        Provider::ClaudeCode => claude_code::launch::contract(),
        Provider::OpenCode => opencode::launch::contract(),
        Provider::Omp => omp::launch::contract(),
    }
}

/// Builds the start command for a provider, validating resume capability.
pub fn start_command(provider: Provider, resume: bool) -> Result<Command, Failure> {
    let contract = launch_contract(provider);
    if resume && !contract.capabilities.supports_resume {
        return Err(Failure::blocked(
            "harness.no_resume",
            format!("`{}` cannot resume a session", contract.binary),
        )
        .expected("a harness whose capabilities include resume")
        .actual("this harness's `supports_resume` is false")
        .fix(FixAction::safe(
            "session.start_fresh",
            "Start a fresh session instead of resuming.",
        )));
    }
    match provider {
        Provider::ClaudeCode => Ok(claude_code::launch::start_command(resume)),
        Provider::OpenCode => Ok(opencode::launch::start_command(resume)),
        Provider::Omp => Ok(omp::launch::start_command(resume)),
    }
}

/// The root key under which MCP servers are configured in the provider config file.
#[must_use]
pub fn mcp_root_key(provider: Provider) -> &'static str {
    match provider {
        Provider::ClaudeCode => claude_code::mcp::ROOT_KEY,
        Provider::OpenCode => opencode::mcp::ROOT_KEY,
        Provider::Omp => omp::mcp::ROOT_KEY,
    }
}

/// Renders a single MCP server definition into provider-native JSON shape.
#[must_use]
pub fn mcp_server_doc(provider: Provider, name: &str, server: &McpServerDef) -> serde_json::Value {
    match provider {
        Provider::ClaudeCode => claude_code::mcp::server_doc(name, server),
        Provider::OpenCode => opencode::mcp::server_doc(name, server),
        Provider::Omp => omp::mcp::server_doc(name, server),
    }
}

/// Returns the managed standalone file artifacts for a provider.
#[must_use]
pub fn managed_artifacts(provider: Provider) -> Vec<ManagedArtifact> {
    match provider {
        Provider::ClaudeCode => claude_code::hook::managed_artifacts(),
        Provider::OpenCode => opencode::hook::managed_artifacts(),
        Provider::Omp => omp::hook::managed_artifacts(),
    }
}

/// One hall directory projected into a session's provider config dir.
///
/// `hall_source` is hall-relative; `config_relative_dest` is relative to
/// the session's `<view_dir>/<config_dir>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionProjection {
    pub hall_source: Utf8PathBuf,
    pub config_relative_dest: Utf8PathBuf,
}

/// Every provider projects its command catalog; the source path is
/// `Provider::commands_dir()`, not a per-provider copy of it. Providers add
/// their own extra projections on top.
#[must_use]
pub fn session_projections(provider: Provider) -> Vec<SessionProjection> {
    let mut projections = vec![SessionProjection {
        hall_source: Utf8PathBuf::from(provider.commands_dir()),
        config_relative_dest: Utf8PathBuf::from("commands"),
    }];
    match provider {
        Provider::ClaudeCode | Provider::OpenCode => {}
        Provider::Omp => projections.extend(omp::session::extra_projections()),
    }
    projections
}

/// Parses provider-specific stdin JSON into a normalized `ToolRequest` and optional cwd.
pub fn parse_tool_request(
    provider: Provider,
    stdin_json: &str,
) -> Result<(ToolRequest, Option<Utf8PathBuf>), Failure> {
    match provider {
        Provider::ClaudeCode => claude_code::guard::parse_tool_request(stdin_json),
        Provider::OpenCode => opencode::guard::parse_tool_request(stdin_json),
        Provider::Omp => omp::guard::parse_tool_request(stdin_json),
    }
}

/// Renders a `GuardDecision` into the provider-specific outcome shape and exit code.
#[must_use]
pub fn render_decision(provider: Provider, decision: &GuardDecision) -> GuardOutcome {
    match provider {
        Provider::ClaudeCode => claude_code::guard::render_decision(decision),
        Provider::OpenCode => opencode::guard::render_decision(decision),
        Provider::Omp => omp::guard::render_decision(decision),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/providers/mod.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/providers/mcp.rs"]
mod mcp_tests;

#[cfg(test)]
#[path = "../../tests/unit/providers/hook.rs"]
mod hook_tests;
