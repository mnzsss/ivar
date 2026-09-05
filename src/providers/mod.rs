//! Closed facade dispatching provider-native behaviors by `Provider`.

pub mod claude_code;
pub mod omp;
pub mod opencode;

use crate::domain::guard::{GuardDecision, GuardOutcome, ToolRequest};
use crate::domain::mcp::{McpServerDef, McpTransport};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::infra::oauth::Tokens;
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
///
/// `mcp_allowlist` carries the hall-qualified names of the MCP servers the
/// manifest declares. Claude Code serialises them into `--settings` so the
/// user is not prompted to approve servers Ivar itself materialised; an empty
/// list is still passed explicitly, so no project MCP inherits approval.
/// Every other provider ignores it and its argv is unchanged.
pub fn start_command(
    provider: Provider,
    resume: bool,
    mcp_allowlist: &[String],
) -> Result<Command, Failure> {
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
        Provider::ClaudeCode => Ok(claude_code::launch::start_command(resume, mcp_allowlist)),
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
///
/// `transport` is the canonical interpretation of the manifest's `type`,
/// already validated by the caller, so each provider renders its own
/// spelling of a value that cannot be anything but `http` or `local`.
#[must_use]
pub fn mcp_server_doc(
    provider: Provider,
    name: &str,
    server: &McpServerDef,
    transport: McpTransport,
) -> serde_json::Value {
    match provider {
        Provider::ClaudeCode => claude_code::mcp::server_doc(name, server, transport),
        Provider::OpenCode => opencode::mcp::server_doc(name, server, transport),
        Provider::Omp => omp::mcp::server_doc(name, server, transport),
    }
}

/// Returns the managed standalone file artifacts for a provider.
#[must_use]
pub fn managed_artifacts(provider: Provider) -> Vec<ManagedArtifact> {
    match provider {
        Provider::ClaudeCode => claude_code::hook::managed_artifacts(),
        Provider::OpenCode => opencode::hook::managed_artifacts(),
        Provider::Omp => omp::managed_artifacts(),
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

/// One server's freshly-exchanged OAuth credential, before any provider
/// has decided how to store it.
///
/// This is what crosses the provider boundary. The on-disk record is a
/// provider's own business: OpenCode turns this into an `mcp-auth.json`
/// entry, and another provider need not have a file at all. `Debug` is
/// redacted — `Tokens` and `ClientInfo` both redact themselves, and this
/// must not become the one place a secret prints.
#[derive(Clone)]
pub struct Credential<'a> {
    pub server_url: &'a str,
    pub client_id: &'a str,
    pub client_secret: Option<&'a str>,
    pub tokens: &'a Tokens,
}

impl std::fmt::Debug for Credential<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("server_url", &self.server_url)
            .field("client_id", &"<redacted>")
            .field("client_secret", &"<redacted>")
            .field("tokens", &"<redacted>")
            .finish()
    }
}

/// Persist a credential in the provider's own store.
///
/// `Ok(false)` means the provider keeps no store of its own and relies on
/// its login command — not a failure, and not something the caller should
/// have to distinguish by provider id.
pub fn install_credentials(
    provider: Provider,
    name: &str,
    credential: &Credential<'_>,
) -> Result<bool, Failure> {
    match provider {
        Provider::ClaudeCode => Ok(false),
        Provider::OpenCode => opencode::auth::install_credentials(name, credential).map(|()| true),
        Provider::Omp => omp::auth::install_credentials(name, credential).map(|()| true),
    }
}

/// Whether an existing entry for `name` would be overwritten.
///
/// `server_url` is what `omp` keys its store by — it stores credentials per
/// MCP endpoint, not per name — and is `None` for a provider that needs none.
/// Claude Code keeps no store Ivar can inspect, so it never reports a
/// conflict: its own login command owns that decision.
pub fn has_credentials(
    provider: Provider,
    name: &str,
    server_url: Option<&str>,
) -> Result<bool, Failure> {
    match provider {
        Provider::ClaudeCode => Ok(false),
        Provider::OpenCode => opencode::auth::has_entry(name),
        Provider::Omp => Ok(server_url.is_some_and(omp::auth::has_entry)),
    }
}

/// The subcommand its login command takes, after the binary, or `None` for a
/// provider that has no MCP login command at all.
///
/// The binary itself comes from `launch_contract(provider).binary` — this
/// returns only the part that differs, so the binary keeps one home.
/// `omp` has no `mcp` subcommand (measured against omp/18.1.8; its auth
/// surface is `omp auth-broker`, which Task 10 owns), so it returns `None`
/// rather than a command that would fail at spawn.
pub fn login_subcommand(provider: Provider) -> Option<[&'static str; 2]> {
    match provider {
        Provider::ClaudeCode => Some(claude_code::auth::LOGIN_SUBCOMMAND),
        Provider::OpenCode => Some(opencode::auth::LOGIN_SUBCOMMAND),
        Provider::Omp => None,
    }
}

/// Confirm the login actually landed, for providers whose exit code lies.
pub fn verify_authenticated(
    provider: Provider,
    name: &str,
    server_url: Option<&str>,
) -> Result<(), Failure> {
    match provider {
        Provider::ClaudeCode => Ok(()),
        Provider::OpenCode => opencode::auth::verify_authenticated(name),
        Provider::Omp => {
            let Some(server_url) = server_url else {
                return Err(Failure::blocked(
                    "omp_auth.missing_server_url",
                    format!(
                        "cannot verify OMP authentication for MCP server `{name}` without a server URL"
                    ),
                )
                .expected("a server URL for OMP credential binding lookup")
                .actual("no server URL provided")
                .fix(FixAction::safe(
                    "mcp.check_config",
                    "Configure a `url` for the MCP server before authenticating with OMP.",
                )));
            };
            omp::auth::verify_authenticated(server_url)
        }
    }
}

/// Reconciles provider-specific active profile commands bridge (e.g. OMP profile commands).
pub fn bridge_sync_commands(
    provider: Provider,
    hall_commands_dir: &camino::Utf8Path,
    command_file_names: &[&str],
    warnings: &mut Vec<crate::error::Warning>,
) {
    match provider {
        Provider::Omp => {
            omp::commands::bridge_sync(hall_commands_dir, command_file_names, warnings)
        }
        Provider::ClaudeCode | Provider::OpenCode => {}
    }
}

/// Removes provider-specific active profile commands bridge for this hall.
pub fn bridge_remove_commands(
    provider: Provider,
    hall_commands_dir: &camino::Utf8Path,
    warnings: &mut Vec<crate::error::Warning>,
) {
    match provider {
        Provider::Omp => omp::commands::bridge_remove(hall_commands_dir, warnings),
        Provider::ClaudeCode | Provider::OpenCode => {}
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

#[cfg(test)]
#[path = "../../tests/unit/providers/extension.rs"]
mod extension_tests;

#[cfg(test)]
#[path = "../../tests/unit/providers/auth.rs"]
mod auth_tests;
