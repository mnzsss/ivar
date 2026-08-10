//! Types for hall-scoped MCP server definitions.
//!
//! The valhalla definition this ports: **MCP** — "Hall-scoped, per-provider
//! MCP server definitions that are part of the Canonical Hall Config.
//! Materialised at the hall root by `ivar sync` (`.mcp.json` for Claude Code,
//! the OpenCode equivalent) and discovered by walk-up from the View Dir, so
//! they apply to every session in the hall — the same shape as the hall skill
//! home. v1 stores only server-side definitions and references secrets via env
//! vars; it does not store secrets."
//!
//! # What lives here
//!
//! Exactly one type: [`McpServerDef`], the per-server definition the manifest
//! carries. The *shape* of the on-disk config is the harness layer's problem —
//! `harness::config` translates this into `.mcp.json` (Claude Code) or
//! `opencode.json` (OpenCode) — because the two harnesses spell the same
//! definition differently (see below). `domain` stays pure: no I/O, no
//! provider knowledge beyond what [`crate::domain::provider`] already owns.
//!
//! # The shape is Claude-Code-first
//!
//! `McpServerDef` models the Claude Code `.mcp.json` entry — `command` and
//! `args` as separate fields, `env` for the environment — because that is the
//! finer-grained shape and the OpenCode form is a lossless translation of it
//! (one `command` array, `environment` for the env map). The canonical shape
//! living here keeps the manifest format stable even though the two harnesses
//! render it differently.
//!
//! # No secrets
//!
//! The only secret-adjacent field is `env`, and it holds *references* — an
//! env var name the harness resolves when it spawns the server — never a
//! secret value. That is the whole v1 boundary: the definitions are committed
//! to `ivar.json`, so they cannot carry credentials.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One MCP server definition: how a harness should spawn (or connect to) one
/// server, and nothing about the secrets it will need at runtime.
///
/// `type_` is the transport: `stdio`, `sse`, or `streamable-http`. A stdio
/// server carries `command` (plus `args`); a remote one carries `url`. The
/// harness materialiser decides how the two spell the same facts on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpServerDef {
    /// The server's name — also the key its config hangs off in the
    /// harness's file. Unique within a hall's manifest.
    pub name: String,
    /// The transport: `stdio`, `sse`, or `streamable-http`.
    #[serde(rename = "type")]
    pub type_: String,
    /// The executable a stdio server is spawned with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Arguments appended to [`Self::command`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// The URL a remote (sse / streamable-http) server is reached at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Env vars the server is spawned with. Values are *references* — names
    /// the harness resolves at runtime — never stored secrets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
}

impl McpServerDef {
    /// Build a definition from its required fields. The optional halves
    /// (`command`, `args`, `url`, `env`) start absent and are set with the
    /// chaining setters below.
    #[must_use]
    pub fn new(name: impl Into<String>, type_: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_: type_.into(),
            command: None,
            args: None,
            url: None,
            env: None,
        }
    }

    /// Set the executable a stdio server is spawned with.
    #[must_use]
    pub fn command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }

    /// Set the arguments appended to the command.
    #[must_use]
    pub fn args(mut self, args: Vec<String>) -> Self {
        self.args = Some(args);
        self
    }

    /// Set the URL a remote server is reached at.
    #[must_use]
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set the env vars the server is spawned with.
    #[must_use]
    pub fn env(mut self, env: BTreeMap<String, String>) -> Self {
        self.env = Some(env);
        self
    }
}

#[cfg(test)]
#[path = "../../tests/unit/domain/mcp.rs"]
mod tests;
