//! `ivar mcp add` — add an HTTP or stdio MCP server to `ivar.json` and materialise harness configs.

use std::io;

use serde::Serialize;

use crate::action::{Ctx, discover_hall, read_manifest};
use crate::domain::mcp::{McpServerDef, McpTransport};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::store::manifest::Manifest;

/// Input arguments for adding an MCP server.
#[derive(Debug, Clone)]
pub struct AddInput {
    /// The server name (e.g. `figma`, `linear`, or a custom identifier).
    pub name: String,
    /// Remote HTTP(S) URL. When omitted with no `command`, `name` is looked up
    /// in [`crate::domain::mcp::MCP_PRESETS`].
    pub url: Option<String>,
    /// The executable of a stdio (`local`) server.
    pub command: Option<String>,
    /// Arguments for `command`.
    pub args: Vec<String>,
}

/// Result of adding an MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpAdded {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    pub changed: bool,
    pub providers: Vec<String>,
    pub next: String,
}

impl McpAdded {
    fn target(&self) -> String {
        describe_target(self.url.as_deref(), self.command.as_deref(), &self.args)
    }
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

impl WriteHuman for McpAdded {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.changed {
            writeln!(
                w,
                "Added MCP server `{}` ({}) for: {}.",
                self.name,
                self.target(),
                self.providers.join(", ")
            )?;
        } else {
            writeln!(
                w,
                "MCP server `{}` already configured with identical definition ({}).",
                self.name,
                self.target()
            )?;
        }
        writeln!(w, "Next: {}", self.next)
    }
}

/// Add an MCP server to the hall manifest and materialise harness configs.
pub fn add(ctx: &Ctx, input: &AddInput) -> Outcome<McpAdded> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    let new_def = super::server_def(
        &input.name,
        input.url.as_deref(),
        input.command.as_deref(),
        &input.args,
    )?;
    let changed = match manifest.mcp_servers().iter().find(|s| s.name == input.name) {
        Some(existing) if super::same_definition(existing, &new_def) => false,
        Some(existing) => return Err(server_conflict(existing, &new_def)),
        None => true,
    };
    let providers = if changed {
        let mut servers = manifest.mcp_servers().to_vec();
        servers.push(new_def.clone());
        let updated = manifest.with_mcp_servers(servers)?;
        Manifest::write(&layout, &updated)?;
        super::materialise_all(&layout, &updated)?
    } else {
        manifest
            .providers()
            .config_owners()
            .map(|p| p.to_string())
            .collect()
    };

    let next = match new_def.transport() {
        Ok(McpTransport::Local) => format!("ivar mcp status {}", input.name),
        _ => format!(
            "ivar mcp auth {} --provider {}",
            input.name,
            manifest.providers().default_provider()
        ),
    };

    Ok(Report::new(McpAdded {
        name: input.name.clone(),
        url: new_def.url,
        command: new_def.command,
        args: new_def.args.unwrap_or_default(),
        changed,
        providers,
        next,
    }))
}

fn describe_def(def: &McpServerDef) -> String {
    match (&def.url, &def.command) {
        (_, Some(command)) => format!("type=`{}`, command=`{command}`", def.type_),
        (url, None) => format!(
            "type=`{}`, url=`{}`",
            def.type_,
            url.as_deref().unwrap_or("<none>")
        ),
    }
}

fn server_conflict(existing: &McpServerDef, new: &McpServerDef) -> Failure {
    Failure::blocked(
        "mcp.server_conflict",
        format!(
            "MCP server `{}` is already defined in ivar.json with a different configuration",
            new.name
        ),
    )
    .expected(format!("existing definition: {}", describe_def(existing)))
    .actual(format!("new definition: {}", describe_def(new)))
    .fix(FixAction::safe(
        "mcp.resolve_conflict",
        "Keep the existing definition or remove it from ivar.json before adding.",
    ))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/mcp/add.rs"]
mod tests;
