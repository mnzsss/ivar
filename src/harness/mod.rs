//! Provider adapters: the code that knows what each harness wants on disk.
//!
//! `domain::provider` holds what a harness *is* — its id, its dotdir, its
//! instruction file. This layer holds what `ivar` *does* about it, which needs
//! I/O and therefore cannot live in `domain`.
//!
//! # What is here now
//!
//! [`config`] — the managed block in each harness's instruction file. That is
//! slice 2's harness work: `ivar sync` materialises it, and removes it for a
//! provider the hall no longer lists.
//!
//! [`commands`] — the embedded catalog of shipped workflow commands, and the
//! reconciliation that materialises them into each provider's command
//! directory. The catalog and every Markdown source are compiled into the
//! binary; this module owns only the command files `ivar-*.md` and leaves every
//! other file in the command directory to the user.
//!
//! [`Harness`] — closed-enum dispatch for MCP auth subcommands (migrating to
//! `providers` in Task 09).
//! # Writing to OpenCode's own credential store
//!
//! [`opencode_auth`] reads and writes
//! `<data_dir>/opencode/mcp-auth.json` — OpenCode's MCP OAuth credential
//! store. An earlier version wrote a pre-provisioned `clientInfo` here that
//! `opencode mcp auth` never consulted; that version was deleted. The writer
//! returns because Ivar now performs the Figma OAuth exchange itself and
//! writes `tokens` into this store — OpenCode reads them at MCP-connect time.
//! `write_entry` never overwrites an existing same-name entry (`R-CONFLICT`,
//! `C-NO-OVERWRITE`); [`opencode_auth::has_tokens`] reports whether the
//! exchange completed, which is still needed because `opencode mcp auth`
//! exits `0` unconditionally (measured 2026-08-26).
//!
//! # Layering
//!
//! `harness` may import `domain`, `infra` and `error`. Not `store` — so paths
//! arrive here already computed by [`crate::store::layout`], which stays the
//! one place that knows the on-disk tree.

pub mod commands;
pub mod config;
pub mod opencode_auth;

use crate::domain::provider::Provider;
use crate::error::Failure;
/// A harness adapter key for MCP auth dispatch (migrating to `providers` in Task 09).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Harness {
    /// Claude Code (`claude` CLI).
    ClaudeCode,
    /// OpenCode (`opencode` CLI).
    OpenCode,
}

impl Harness {
    /// The harness for a provider, or a `Blocked` failure naming the
    /// provider id.
    pub fn for_provider(provider: Provider) -> Result<Self, Failure> {
        match provider {
            Provider::ClaudeCode => Ok(Self::ClaudeCode),
            Provider::OpenCode => Ok(Self::OpenCode),
            Provider::Omp => Err(Failure::blocked(
                "harness.unsupported",
                "OMP harness launch is not yet configured",
            )),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/harness/mod.rs"]
mod tests;
