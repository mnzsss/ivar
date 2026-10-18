//! `ivar mcp` — declare, authenticate and inspect hall MCP servers.
//!
//! [`auth`]'s module doc explains the three steps and why a registration is never reported as an authentication.

pub mod add;
pub mod auth;
pub mod edit;
pub mod remove;
pub mod status;

use crate::domain::mcp::{MCP_PRESETS, McpServerDef, McpValidationError, mcp_preset};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::harness::config;
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

pub(crate) fn server_def(
    name: &str,
    url: Option<&str>,
    command: Option<&str>,
    args: &[String],
) -> Result<McpServerDef, Failure> {
    let def = match (url, command) {
        (_, Some(command)) => {
            let def = McpServerDef::new(name, "local").command(command);
            if args.is_empty() {
                def
            } else {
                def.args(args.to_vec())
            }
        }
        (Some(url), None) => McpServerDef::new(name, "http").url(url),
        (None, None) => match mcp_preset(name) {
            Some(preset) => McpServerDef::new(name, "http").url(preset.url),
            None => return Err(unknown_preset(name)),
        },
    };
    def.validate().map_err(|error| invalid_definition(&error))?;
    Ok(def)
}

/// OAuth metadata is written by `ivar mcp auth` after declaration, so it is
/// not part of what makes two definitions the same.
pub(crate) fn same_definition(a: &McpServerDef, b: &McpServerDef) -> bool {
    a.type_ == b.type_
        && a.url == b.url
        && a.command == b.command
        && a.args == b.args
        && a.env == b.env
}

pub(crate) fn materialise_all(
    layout: &Layout,
    manifest: &Manifest,
) -> Result<Vec<String>, Failure> {
    for provider in manifest.providers().config_owners() {
        config::materialise_mcp(
            &layout.mcp_config(&provider),
            provider,
            manifest.mcp_servers(),
            manifest.name(),
        )?;
    }
    Ok(owner_ids(manifest))
}

pub(crate) fn owner_ids(manifest: &Manifest) -> Vec<String> {
    manifest
        .providers()
        .config_owners()
        .map(|p| p.to_string())
        .collect()
}

pub(crate) fn describe_target(url: Option<&str>, command: Option<&str>, args: &[String]) -> String {
    match (url, command) {
        (Some(url), _) => url.to_owned(),
        (None, Some(command)) => std::iter::once(command)
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        (None, None) => String::new(),
    }
}

pub(crate) fn describe_def(def: &McpServerDef) -> String {
    format!(
        "type=`{}`, target=`{}`",
        def.type_,
        describe_target(
            def.url.as_deref(),
            def.command.as_deref(),
            def.args.as_deref().unwrap_or_default()
        )
    )
}

pub(crate) fn client_secret_env(server: &McpServerDef) -> Option<&String> {
    server.oauth.as_ref()?.client_secret_env.as_ref()
}

/// The client secret `gone` referenced, when no `remaining` server shares it and it is stored.
pub(crate) fn orphan_secret(
    gone: &McpServerDef,
    remaining: &[McpServerDef],
    secrets: &McpSecrets,
) -> Option<String> {
    client_secret_env(gone)
        .filter(|var| !remaining.iter().any(|s| client_secret_env(s) == Some(*var)))
        .filter(|var| secrets.contains_key(var))
        .cloned()
}

fn unknown_preset(name: &str) -> Failure {
    let preset_names: Vec<&str> = MCP_PRESETS.iter().map(|p| p.name).collect();
    Failure::blocked("mcp.unknown_preset", format!("unknown MCP preset `{name}`"))
        .expected(format!(
            "one of the known presets ({}) or pass `--url <URL>`",
            preset_names.join(", ")
        ))
        .actual(format!("`{name}` has no preset and no `--url` was given"))
        .fix(FixAction::safe(
            "mcp.use_preset_or_url",
            format!(
                "Run with a known preset, provide `--url`: `ivar mcp add {name} --url <URL>`, \
                 or provide `--command`: `ivar mcp add {name} --command <EXECUTABLE>`"
            ),
        ))
}

fn invalid_definition(error: &McpValidationError) -> Failure {
    Failure::blocked(
        "manifest.invalid_mcp_definition",
        format!("invalid MCP server definition: {error}"),
    )
    .expected("a valid definition: an `http` URL, or a `local` command")
    .actual(format!("validation failed: {error}"))
    .fix(FixAction::safe(
        "mcp.fix_definition",
        "Specify a valid HTTP URL with `--url`, or an executable with `--command`.",
    ))
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
