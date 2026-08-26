//! Read-only access to OpenCode's own MCP OAuth credential store —
//! `<data_dir>/opencode/mcp-auth.json`.
//!
//! # This store is the wrong place to *write* a client
//!
//! An earlier version of this module wrote a pre-provisioned `clientInfo`
//! here, on the strength of a `clientInformation()` resolution order read out
//! of the OpenCode binary. Measured on 2026-08-26: `opencode mcp auth` never
//! reads this file for its OAuth client at all — it resolves that from
//! `opencode.json` only. That version was deleted rather than kept as dead
//! weight (see `plans/ivar-mcp-auth/analysis.md`); pre-registration now
//! reaches OpenCode through `harness::config`'s materialised `opencode.json`,
//! driven by `McpServerDef.oauth`. Do not re-add a writer here — nothing
//! this crate does reads a client back out of this file.
//!
//! # It *is* the right place to read whether authentication happened
//!
//! `opencode mcp auth` exits `0` unconditionally — measured against a server
//! name that does not exist, and measured while it printed `Authentication
//! failed` to the terminal. The exit status cannot carry `R-HONEST` for this
//! provider. What this file *does* reflect, reliably, is whether a token
//! exchange actually completed: a successful `opencode mcp auth` writes a
//! `tokens` object under the server's name here. [`has_tokens`] is the one
//! thing this module does.
//!
//! # Layering
//!
//! `harness` may import `infra` — [`crate::infra::fs::data_dir`] resolves the
//! base directory, [`crate::infra::json::read`] parses the file. No `store`
//! import: the caller hands this module nothing but a server name.

use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;
use std::collections::BTreeMap;

use crate::error::Failure;
use crate::infra::{fs, json};

/// One server's entry in `mcp-auth.json`. Only the field this module reads —
/// OpenCode's own store carries more, and none of the rest is this module's
/// business.
#[derive(Debug, Deserialize)]
struct StoredAuth {
    /// Present once a token exchange actually completed for this server.
    #[serde(default)]
    tokens: Option<serde_json::Value>,
}

/// `mcp-auth.json`'s path under `data_dir`. Split out from [`has_tokens`] so
/// the path arithmetic is a plain, deterministic function — no environment
/// variable to set up to exercise it.
fn auth_path_under(data_dir: &Utf8Path) -> Utf8PathBuf {
    data_dir.join("opencode").join("mcp-auth.json")
}

/// Whether OpenCode's own store shows a completed token exchange for
/// `server_name`.
///
/// `Ok(false)` for a missing file, a missing entry, or an entry with no
/// `tokens` — none of those are errors, they are simply "not authenticated
/// yet". An error here means the file exists but could not be read as JSON.
pub fn has_tokens(server_name: &str) -> Result<bool, Failure> {
    has_tokens_under(&fs::data_dir()?, server_name)
}

/// [`has_tokens`], parameterised on the data directory so a test can point it
/// at a temporary file instead of resolving `$XDG_DATA_HOME`/`$HOME`.
fn has_tokens_under(data_dir: &Utf8Path, server_name: &str) -> Result<bool, Failure> {
    let path = auth_path_under(data_dir);
    let store: Option<BTreeMap<String, StoredAuth>> = json::read(&path)?;
    Ok(store
        .and_then(|servers| servers.get(server_name).map(|entry| entry.tokens.is_some()))
        .unwrap_or(false))
}

#[cfg(test)]
#[path = "../../tests/unit/harness/opencode_auth.rs"]
mod tests;
