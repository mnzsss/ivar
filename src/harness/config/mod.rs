//! The hall's MCP server config materialisation, plus the shared session
//! bootstrap block.
//!
//! The canonical hall instructions and the provider root aliases live in
//! [`instructions`] — this module's job is everything else `harness::config`
//! materialises: the MCP server definitions at the hall root, and the
//! session bootstrap block (see [`session`]).
//!
//! # MCP config materialisation: one key at a time
//!
//! The hall's MCP server definitions materialise at the hall root — `.mcp.json`
//! for Claude Code, `opencode.json` for OpenCode — discovered by walk-up from
//! every session's View Dir. [`materialise_mcp`] and [`remove_mcp`] apply the
//! "the file belongs to the user" rule with a JSON key standing in for the
//! marker pair:
//!
//! - `.mcp.json` is *exclusively* an MCP file, so `ivar` owns it wholesale.
//! - `opencode.json` is OpenCode's **general** config — model, permissions,
//!   MCP all live there. `ivar` owns exactly the `mcp` key: the materialiser
//!   merges, replacing that key and leaving every other key the user wrote
//!   untouched, and never clobbers a file it cannot parse as a JSON object.
//!
//! The two harnesses spell the same definition differently, and the
//! translation lives here: Claude Code's `mcpServers` entries keep
//! `command`/`args`/`env` as separate fields; OpenCode's `mcp` entries turn
//! `stdio` into `local` (with `command` as one array) and `sse`/`streamable-http`
//! into `remote`, and rename `env` to `environment`. The `$schema` key OpenCode
//! expects accompanies its `mcp` key.
//!
//! # Idempotence is checked, not assumed
//!
//! [`materialise_mcp`] compares before writing and reports
//! [`Change::Unchanged`] when the content already matches. That is not an
//! optimisation. `ivar sync` is
//! what people run after every `git pull`; a version that rewrote the file each
//! time would put a spurious modification in `git status` on every run, and a
//! tool that dirties your working tree for no reason is a tool you stop
//! running.
//!
//! # Reference
//!
//! The OpenCode `$schema` URL and the `mcp` key shape come from OpenCode's own
//! docs (`opencode.ai/docs/config`, `opencode.ai/docs/mcp-servers`), the same
//! sources
//! [`Provider::mcp_config_path`](crate::domain::provider::Provider::mcp_config_path)
//! cites.

use crate::error::{Failure, FixAction};
use crate::infra::json;

mod mcp;
mod settings;
pub(crate) mod session;

pub mod instructions;

pub use instructions::{Change, MANAGED_END, MANAGED_START, build_block, materialise, remove};
pub use mcp::{OAUTH_REDIRECT_URI, materialise_mcp, remove_mcp};
pub use settings::{materialise_settings, remove_settings};

/// Everything that can go wrong maintaining an MCP config: it could not be
/// parsed, or it parsed as something the `mcp` key cannot merge into.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Something failed reading, writing, or serialising an MCP config — the
    /// wrapped error is `infra::json`'s own, which already distinguishes the
    /// mechanical cause.
    #[error("could not maintain the MCP config `{path}`: {source}")]
    Mcp {
        path: camino::Utf8PathBuf,
        #[source]
        source: json::Error,
    },
    /// The MCP config parsed as JSON but is not an object, so there is no safe
    /// way to merge the `mcp` key into it — and `ivar` will not invent one.
    #[error("`{path}` is not a JSON object; ivar will not overwrite it")]
    McpNotObject { path: camino::Utf8PathBuf },
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        match error {
            Error::Mcp { path, source } => {
                let failure: Failure = source.into();
                failure.fix(FixAction::safe(
                    "harness.check_mcp_config",
                    format!(
                        "Check that `{path}` is valid JSON and writable, then run `ivar sync` again."
                    ),
                ))
            }
            Error::McpNotObject { path } => Failure::blocked(
                "harness.mcp_not_an_object",
                format!("`{path}` is not a JSON object"),
            )
            .expected("a JSON object at the hall root")
            .actual("some other JSON shape")
            .fix(FixAction::safe(
                "harness.fix_mcp_config",
                format!("Make `{path}` a JSON object (or remove it), then run `ivar sync` again."),
            )),
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/harness/config.rs"]
mod tests;
