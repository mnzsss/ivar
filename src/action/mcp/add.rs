//! `ivar mcp add` — add an HTTP MCP server to `ivar.json` and materialise harness configs.

use std::io;

use serde::Serialize;

use crate::action::{Ctx, discover_hall, read_manifest};
use crate::domain::mcp::{MCP_PRESETS, McpServerDef, mcp_preset};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::harness::config;
use crate::store::manifest::Manifest;

/// Input arguments for adding an MCP server.
#[derive(Debug, Clone)]
pub struct AddInput {
    /// The server name (e.g. `figma`, `linear`, or a custom identifier).
    pub name: String,
    /// Remote HTTP(S) URL. When omitted, [`name`] is looked up in [`MCP_PRESETS`].
    pub url: Option<String>,
}

/// Result of adding an MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpAdded {
    pub name: String,
    pub url: String,
    pub changed: bool,
    pub providers: Vec<String>,
    pub next: String,
}

impl WriteHuman for McpAdded {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.changed {
            let provider_list = self.providers.join(", ");
            writeln!(
                w,
                "Added MCP server `{}` ({}) for: {}.",
                self.name, self.url, provider_list
            )?;
            writeln!(w, "Next: {}", self.next)
        } else {
            writeln!(
                w,
                "MCP server `{}` already configured with identical definition ({}).",
                self.name, self.url
            )?;
            writeln!(w, "Next: {}", self.next)
        }
    }
}

/// Add an MCP server to the hall manifest and materialise harness configs.
pub fn add(ctx: &Ctx, input: &AddInput) -> Outcome<McpAdded> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    let url = match (&input.url, mcp_preset(&input.name)) {
        (Some(url), _) => url.clone(),
        (None, Some(preset)) => preset.url.to_owned(),
        (None, None) => {
            let preset_names: Vec<&str> = MCP_PRESETS.iter().map(|p| p.name).collect();
            return Err(Failure::blocked(
                "mcp.unknown_preset",
                format!("unknown MCP preset `{}`", input.name),
            )
            .expected(format!(
                "one of the known presets ({}) or pass `--url <URL>`",
                preset_names.join(", ")
            ))
            .actual(format!(
                "`{}` has no preset and no `--url` was given",
                input.name
            ))
            .fix(FixAction::safe(
                "mcp.use_preset_or_url",
                format!(
                    "Run with a known preset or provide `--url`: `ivar mcp add {} --url <URL>`",
                    input.name
                ),
            )));
        }
    };

    let new_def = McpServerDef::new(&input.name, "http").url(&url);
    new_def.validate().map_err(|e| {
        Failure::blocked(
            "manifest.invalid_mcp_definition",
            format!("invalid MCP server definition: {e}"),
        )
        .expected("a valid HTTP URL starting with http:// or https://")
        .actual(format!("validation failed: {e}"))
        .fix(FixAction::safe("mcp.fix_url", "Specify a valid HTTP URL."))
    })?;

    let existing = manifest.mcp_servers().iter().find(|s| s.name == input.name);

    let mut updated_servers: Vec<McpServerDef> = manifest.mcp_servers().to_vec();
    let changed = if let Some(existing_def) = existing {
        // Authentication adds OAuth metadata after declaration; it is not a conflict.
        if existing_def.type_ == new_def.type_
            && existing_def.url == new_def.url
            && existing_def.command == new_def.command
            && existing_def.args == new_def.args
            && existing_def.env == new_def.env
        {
            false
        } else {
            return Err(Failure::blocked(
                "mcp.server_conflict",
                format!(
                    "MCP server `{}` is already defined in ivar.json with a different configuration",
                    input.name
                ),
            )
            .expected(format!(
                "existing definition: type=`{}`, url=`{}`",
                existing_def.type_,
                existing_def.url.as_deref().unwrap_or("<none>")
            ))
            .actual(format!("new definition: type=`http`, url=`{url}`"))
            .fix(FixAction::safe(
                "mcp.resolve_conflict",
                "Keep the existing definition or remove it from ivar.json before adding.",
            )));
        }
    } else {
        updated_servers.push(new_def);
        true
    };

    let default_provider = manifest.providers().default_provider();
    let available_providers = manifest.providers().available().to_vec();
    let provider_names: Vec<String> = available_providers.iter().map(|p| p.to_string()).collect();

    if changed {
        let updated_manifest = manifest.with_mcp_servers(updated_servers)?;
        Manifest::write(&layout, &updated_manifest)?;

        for provider in &available_providers {
            let mcp_path = layout.mcp_config(provider);
            config::materialise_mcp(
                &mcp_path,
                *provider,
                updated_manifest.mcp_servers(),
                updated_manifest.name(),
            )?;
        }
    }

    let next = format!(
        "ivar mcp auth {} --provider {}",
        input.name, default_provider
    );

    Ok(Report::new(McpAdded {
        name: input.name.clone(),
        url,
        changed,
        providers: provider_names,
        next,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/mcp/add.rs"]
mod tests;
