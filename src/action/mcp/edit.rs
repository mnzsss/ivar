//! `ivar mcp edit` — replace a declared MCP server's definition and rematerialise harness configs.

use std::io;

use serde::Serialize;

use super::describe_target;
use crate::action::{Ctx, discover_hall, read_manifest};
use crate::domain::mcp::McpServerDef;
use crate::error::{Outcome, Report, WriteHuman};
use crate::store::manifest::Manifest;

/// Input arguments for editing an MCP server.
#[derive(Debug, Clone)]
pub struct EditInput {
    /// The declared server's name.
    pub name: String,
    /// The new HTTP(S) URL.
    pub url: Option<String>,
    /// The executable of a stdio (`local`) server.
    pub command: Option<String>,
    /// Arguments for `command`.
    pub args: Vec<String>,
}

/// Result of editing an MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpEdited {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    pub changed: bool,
    pub oauth_dropped: bool,
    pub providers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

impl McpEdited {
    fn from_def(
        def: &McpServerDef,
        changed: bool,
        oauth_dropped: bool,
        providers: Vec<String>,
    ) -> Self {
        Self {
            name: def.name.clone(),
            url: def.url.clone(),
            command: def.command.clone(),
            args: def.args.clone().unwrap_or_default(),
            changed,
            oauth_dropped,
            providers,
            next: oauth_dropped.then(|| format!("ivar mcp auth {}", def.name)),
        }
    }
}

impl WriteHuman for McpEdited {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let target = describe_target(self.url.as_deref(), self.command.as_deref(), &self.args);
        if self.changed {
            writeln!(
                w,
                "Updated MCP server `{}` ({target}) for: {}.",
                self.name,
                self.providers.join(", ")
            )?;
        } else {
            writeln!(
                w,
                "MCP server `{}` already has this definition ({target}).",
                self.name
            )?;
        }
        if let Some(next) = &self.next {
            writeln!(w, "Next: {next}")?;
        }
        Ok(())
    }
}

/// Replace a declared MCP server's definition and materialise harness configs.
pub fn edit(ctx: &Ctx, input: &EditInput) -> Outcome<McpEdited> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let existing = super::resolve_server(&manifest, &input.name)?;
    let mut new_def = super::server_def(
        &input.name,
        input.url.as_deref(),
        input.command.as_deref(),
        &input.args,
    )?;
    new_def.env.clone_from(&existing.env);
    if super::same_definition(existing, &new_def) {
        return Ok(Report::new(McpEdited::from_def(
            existing,
            false,
            false,
            super::owner_ids(&manifest),
        )));
    }
    let same_origin = existing.type_ == new_def.type_ && existing.url == new_def.url;
    let oauth_dropped = existing.oauth.is_some() && !same_origin;
    if same_origin {
        new_def.oauth.clone_from(&existing.oauth);
    }
    let servers = manifest
        .mcp_servers()
        .iter()
        .map(|s| {
            if s.name == input.name {
                new_def.clone()
            } else {
                s.clone()
            }
        })
        .collect();
    let updated = manifest.with_mcp_servers(servers)?;
    Manifest::write(&layout, &updated)?;
    let providers = super::materialise_all(&layout, &updated)?;
    Ok(Report::new(McpEdited::from_def(
        &new_def,
        true,
        oauth_dropped,
        providers,
    )))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/mcp/edit.rs"]
mod tests;
