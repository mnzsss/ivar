//! Closed facade dispatching provider-native behaviors by `Provider`.

pub mod claude_code;
pub mod omp;
pub mod opencode;
pub(crate) mod search;

use crate::domain::guard::{GuardDecision, GuardOutcome, ToolRequest};
use crate::domain::mcp::{CredentialState, McpServerDef, McpTransport};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::infra::fs;
use crate::infra::oauth::Tokens;
use crate::infra::proc::Command;
use camino::Utf8PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
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

/// The plain `<binary> [--continue]` shape most providers start with — only
/// Claude Code's start command carries anything more (its MCP allowlist).
#[must_use]
fn resumable_start_command(binary: &'static str, resume: bool) -> Command {
    let command = Command::new(binary);
    if resume {
        command.arg("--continue")
    } else {
        command
    }
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
///
/// # Errors
///
/// Returns [`Failure`] if `resume` is requested but `provider`'s
/// launch contract does not support it.
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

/// A hall-root MCP file the provider also reads, from which sync removes this
/// hall's servers.
#[must_use]
pub fn legacy_mcp_config(provider: Provider) -> Option<&'static str> {
    match provider {
        Provider::Omp => Some(omp::mcp::LEGACY_ROOT_CONFIG),
        Provider::ClaudeCode | Provider::OpenCode => None,
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

/// Extracts the search text from a Grep/Glob/Bash tool call, for the
/// guard's skip/follow-up heuristics. `None` for every other tool, and
/// for a Bash command that isn't a search.
pub(crate) fn extract_search_pattern(tool: &str, input: &serde_json::Value) -> Option<String> {
    let raw = match tool.to_ascii_lowercase().as_str() {
        "grep" | "glob" => input.get("pattern")?.as_str()?.to_owned(),
        "bash" => search::bash_search_command(input.get("command")?.as_str()?)?,
        _ => return None,
    };
    Some(crate::domain::graph::truncate_for_storage(&raw))
}

/// Parses provider-specific stdin JSON into a normalized `ToolRequest` and optional cwd.
/// # Errors
///
/// Returns [`Failure`] if `stdin_json` is not valid for `provider`'s
/// tool-request shape.
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

/// Renders a `GuardDecision` into the provider-specific outcome shape and
/// exit code. `context` (repository instructions) rides only on an allow.
#[must_use]
pub fn render_decision(
    provider: Provider,
    decision: &GuardDecision,
    context: Option<&str>,
) -> GuardOutcome {
    match provider {
        Provider::ClaudeCode => claude_code::guard::render_decision(decision, context),
        Provider::OpenCode => opencode::guard::render_decision(decision, context),
        Provider::Omp => omp::guard::render_decision(decision, context),
    }
}

/// Renders context with no decision, for Claude Code's extra slice hook
/// entries. Always exits 0.
#[must_use]
pub fn render_context(provider: Provider, context: Option<&str>) -> GuardOutcome {
    match provider {
        Provider::ClaudeCode => claude_code::guard::render_context(context),
        Provider::OpenCode | Provider::Omp => GuardOutcome {
            body: context.unwrap_or_default().to_owned(),
            exit_zero: true,
        },
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
///
/// # Errors
///
/// Returns [`Failure`] if the provider's credential store cannot be
/// written.
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

/// Inspect the credential state for a given provider × server name × URL tuple.
#[must_use]
pub fn credential_state(provider: Provider, name: &str, server_url: &str) -> CredentialState {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    #[allow(clippy::cast_possible_truncation)]
    let now_ms = now.as_millis() as u64;
    let now_s = now.as_secs_f64();

    match provider {
        Provider::ClaudeCode => match claude_code::auth::config_dir() {
            Ok(dir) => claude_code::auth::credential_state_under(&dir, name, server_url, now_ms),
            Err(_) => CredentialState::Unknown,
        },
        Provider::OpenCode => match fs::data_dir() {
            Ok(dir) => opencode::auth::credential_state_under(&dir, name, now_s),
            Err(_) => CredentialState::Unknown,
        },
        Provider::Omp => {
            if omp::auth::has_entry(name) {
                CredentialState::Authenticated
            } else {
                CredentialState::Missing
            }
        }
    }
}

/// Probe the provider harness's live MCP status in the given hall `cwd`.
///
/// Returns `None` for providers without a live probe command (e.g. OMP).
/// For Claude Code (`claude mcp list`) and OpenCode (`opencode mcp list`),
/// runs the command in `cwd`, captures stdout, and parses the output into
/// a map of materialised server name -> CredentialState.
pub fn live_states(
    provider: Provider,
    cwd: &camino::Utf8Path,
) -> Option<Result<std::collections::BTreeMap<String, CredentialState>, Failure>> {
    match provider {
        Provider::Omp => None,
        Provider::ClaudeCode => {
            let cmd = Command::new("claude").args(["mcp", "list"]).cwd(cwd);
            let output = match crate::infra::proc::capture(&cmd) {
                Ok(out) => out,
                Err(e) => {
                    return Some(Err(Failure::failed(
                        "provider.claude_mcp_list_failed",
                        format!("could not run `claude mcp list`: {e}"),
                    )));
                }
            };
            Some(Ok(claude_code::live::parse_mcp_list(&output.stdout)))
        }
        Provider::OpenCode => {
            let cmd = Command::new("opencode").args(["mcp", "list"]).cwd(cwd);
            let output = match crate::infra::proc::capture(&cmd) {
                Ok(out) => out,
                Err(e) => {
                    return Some(Err(Failure::failed(
                        "provider.opencode_mcp_list_failed",
                        format!("could not run `opencode mcp list`: {e}"),
                    )));
                }
            };
            Some(Ok(opencode::live::parse_mcp_list(&output.stdout)))
        }
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
///
/// # Errors
///
/// Returns [`Failure`] if the provider's own verification fails.
pub fn verify_authenticated(provider: Provider, name: &str) -> Result<(), Failure> {
    match provider {
        Provider::ClaudeCode => Ok(()),
        Provider::OpenCode => opencode::auth::verify_authenticated(name),
        Provider::Omp => omp::auth::verify_authenticated(name),
    }
}

/// Resolves user home directory from `HOME` (or `USERPROFILE` on Windows).
///
/// # Errors
///
/// Returns [`Failure`] if no home directory variable resolves to an
/// absolute path.
pub fn user_home_from(
    home: Option<String>,
    userprofile: Option<String>,
    os: &str,
) -> Result<Utf8PathBuf, Failure> {
    if let Some(path) = home.filter(|h| !h.is_empty()) {
        let p = Utf8PathBuf::from(path);
        if p.is_absolute() {
            return Ok(p);
        }
    }
    if os == "windows"
        && let Some(path) = userprofile.filter(|u| !u.is_empty())
    {
        let p = Utf8PathBuf::from(&path);
        // On Windows (or cross-platform test), path starting with "C:\" or "\" or "/" is absolute
        if p.is_absolute() || path.chars().nth(1) == Some(':') {
            return Ok(p);
        }
    }
    Err(
        Failure::failed("fs.user_home", "could not resolve user home directory")
            .expected("$HOME (or on Windows, %USERPROFILE%) set to an absolute path")
            .actual("no home directory variable resolved to an absolute path")
            .fix(FixAction::safe(
                "fs.set_home",
                "Set $HOME or %USERPROFILE% to an absolute path.",
            )),
    )
}

/// Resolves the user's home directory from the live process environment.
///
/// # Errors
///
/// Returns [`Failure`] if no home directory variable resolves to an absolute path.
pub fn user_home() -> Result<Utf8PathBuf, Failure> {
    user_home_from(
        std::env::var("HOME").ok(),
        std::env::var("USERPROFILE").ok(),
        std::env::consts::OS,
    )
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
