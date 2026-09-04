//! Read and write access to OpenCode's MCP OAuth credential store —
//! `<data_dir>/opencode/mcp-auth.json`.
//!
//! # Why Ivar writes to this store
//!
//! Earlier versions of this module deliberately avoided writing here.
//! That guidance was correct for the old design: measured on 2026-08-26,
//! `opencode mcp auth` never reads `mcp-auth.json` for its OAuth client
//! at all — it resolves that from `opencode.json` only. A pre-provisioned
//! `clientInfo` written here reached nothing.
//!
//! That reasoning no longer applies. In the new flow Ivar performs the
//! Figma OAuth exchange itself (`R-FIGMA-FLOW`) and writes the resulting
//! tokens into this store. OpenCode *does* read tokens from here at
//! MCP-connect time — this is exactly what the community workaround
//! (`gberaudo/opencode-mcp-figma`) writes. The writer returns, justified
//! by `R-PERSIST` and `R-HONEST`.
//!
//! # Conflict detection
//!
//! `write_entry` never overwrites an existing same-name entry (`R-CONFLICT`,
//! `C-NO-OVERWRITE`). If the key already exists, the write is aborted with
//! a `Failure::blocked` identifying the server name and store path. The
//! user must remove it explicitly.
//!
//! # Secrets
//!
//! Client secrets and tokens are never exposed in `Debug` or error
//! messages. The [`Entry`] and [`ClientInfo`] types implement redacted
//! `Debug` to prevent accidental leakage.
//!
//! # Layering
//!
//! `providers` may import `domain`, `infra`, and `error`. Not `store`,
//! not `action`, not `harness`.

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use std::collections::BTreeMap;

use crate::error::{Failure, FixAction};
use crate::infra::oauth::Tokens;
use crate::infra::{fs, json};
use crate::providers::Credential;

pub(crate) const LOGIN_SUBCOMMAND: [&str; 2] = ["mcp", "auth"];

/// Client registration info stored alongside tokens in OpenCode's
/// `mcp-auth.json`. Values match OpenCode's `ClientInfo` schema:
/// `clientId`, optional `clientSecret`, optional `clientSecretExpiresAt`.
///
/// `Debug` is redacted — secrets must never appear in logs or diagnostics.
#[derive(Clone)]
pub(crate) struct ClientInfo {
    pub(crate) client_id: String,
    pub(crate) client_secret: Option<String>,
    pub(crate) client_secret_expires_at: Option<f64>,
}

impl std::fmt::Debug for ClientInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientInfo")
            .field("client_id", &"<redacted>")
            .field("client_secret", &"<redacted>")
            .field("client_secret_expires_at", &self.client_secret_expires_at)
            .finish()
    }
}

/// The complete OpenCode-compatible entry written to `mcp-auth.json`.
///
/// Contains exactly: `serverUrl`, `clientInfo`, `tokens`. The OpenCode
/// camelCase schema is handled by `#[serde(rename_all = "camelCase")]`.
///
/// `Debug` is redacted — tokens and secrets must never appear in logs.
#[derive(Clone)]
pub(crate) struct Entry {
    pub(crate) server_url: String,
    pub(crate) client_info: ClientInfo,
    pub(crate) tokens: Tokens,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry")
            .field("server_url", &self.server_url)
            .field("client_info", &self.client_info)
            .field("tokens", &"<redacted>")
            .finish()
    }
}

/// The on-disk shape of a single entry in `mcp-auth.json`, serialised with
/// OpenCode's camelCase keys.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreEntry {
    server_url: String,
    client_info: StoreClientInfo,
    tokens: Tokens,
}

/// The on-disk `clientInfo` shape — only the fields that are always
/// present. `serde` skips `None` fields so the output matches OpenCode's
/// expected schema.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreClientInfo {
    client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_secret_expires_at: Option<f64>,
}

/// `mcp-auth.json`'s path under `data_dir`. Split out so the path
/// arithmetic is a plain, deterministic function — no environment
/// variable to set up to exercise it.
pub(crate) fn store_path(data_dir: &Utf8Path) -> Utf8PathBuf {
    data_dir.join("opencode").join("mcp-auth.json")
}

/// Read the raw store map from the auth file. Absent file = empty map.
/// Invalid JSON = error.
pub(crate) fn read_map_under(
    data_dir: &Utf8Path,
) -> Result<BTreeMap<String, serde_json::Value>, Failure> {
    let path = store_path(data_dir);
    let store: Option<BTreeMap<String, serde_json::Value>> = json::read(&path)?;
    Ok(store.unwrap_or_default())
}

/// Whether the store contains any entry (including one with only
/// `codeVerifier`, `{}`, `clientInfo`, etc.) under `server_name`.
///
/// `Ok(false)` for a missing file or a missing entry — those are not
/// errors. An error means the file exists but could not be parsed.
pub(crate) fn has_entry(server_name: &str) -> Result<bool, Failure> {
    has_entry_under(&fs::data_dir()?, server_name)
}

/// [`has_entry`], parameterised on the data directory.
pub(crate) fn has_entry_under(data_dir: &Utf8Path, server_name: &str) -> Result<bool, Failure> {
    let map = read_map_under(data_dir)?;
    Ok(map.contains_key(server_name))
}

/// Whether OpenCode's own store shows a completed token exchange for
/// `server_name`.
///
/// `Ok(false)` for a missing file, a missing entry, or an entry with no
/// `tokens` — none of those are errors, they are simply "not authenticated
/// yet". An error here means the file exists but could not be read as JSON.
#[allow(dead_code)]
pub(crate) fn has_tokens(server_name: &str) -> Result<bool, Failure> {
    has_tokens_under(&fs::data_dir()?, server_name)
}

/// [`has_tokens`], parameterised on the data directory.
pub(crate) fn has_tokens_under(data_dir: &Utf8Path, server_name: &str) -> Result<bool, Failure> {
    let map = read_map_under(data_dir)?;
    Ok(map
        .get(server_name)
        .and_then(|value| value.get("tokens"))
        .is_some_and(serde_json::Value::is_object))
}

/// Write an `Entry` into the store under `server_name`, preserving every
/// existing unrelated entry.
///
/// # Conflict check
///
/// If `server_name` already exists in the store, this returns
/// `Failure::blocked` (`R-CONFLICT`, `C-NO-OVERWRITE`). The entry is
/// never overwritten — the user must remove it explicitly.
///
/// # Atomicity
///
/// The write uses [`fs::write_sensitive_atomic`] with mode `0600` (Unix),
/// so a crash never leaves a half-written file and every unrelated entry
/// is preserved.
#[allow(dead_code)]
pub(crate) fn write_entry(server_name: &str, entry: &Entry) -> Result<(), Failure> {
    write_entry_under(&fs::data_dir()?, server_name, entry)
}

/// [`write_entry`], parameterised on the data directory.
pub(crate) fn write_entry_under(
    data_dir: &Utf8Path,
    server_name: &str,
    entry: &Entry,
) -> Result<(), Failure> {
    let path = store_path(data_dir);
    let mut map = read_map_under(data_dir)?;

    if map.contains_key(server_name) {
        return Err(Failure::blocked(
            "opencode_auth.conflict",
            format!("the store at {path} already has an entry for \"{server_name}\""),
        )
        .expected("no existing entry for this server name")
        .actual("an entry already exists under this key")
        .fix(FixAction::unsafe_(
            "opencode_auth.remove_entry",
            format!(
                "Remove the \"{server_name}\" entry from {path} explicitly before re-authenticating."
            ),
        )));
    }

    let store_entry = StoreEntry {
        server_url: entry.server_url.clone(),
        client_info: StoreClientInfo {
            client_id: entry.client_info.client_id.clone(),
            client_secret: entry.client_info.client_secret.clone(),
            client_secret_expires_at: entry.client_info.client_secret_expires_at,
        },
        tokens: entry.tokens.clone(),
    };

    let entry_value = serde_json::to_value(store_entry).map_err(|e| {
        Failure::failed(
            "opencode_auth.serialize",
            format!("could not serialize credential entry: {e}"),
        )
    })?;
    map.insert(server_name.to_owned(), entry_value);

    let full_bytes = json::to_canonical_string(&map).map_err(Failure::from)?;

    fs::write_sensitive_atomic(&path, full_bytes.as_bytes())?;

    Ok(())
}

pub(crate) fn install_credentials(name: &str, credential: &Credential<'_>) -> Result<(), Failure> {
    install_credentials_under(&fs::data_dir()?, name, credential)
}

pub(crate) fn install_credentials_under(
    data_dir: &Utf8Path,
    name: &str,
    credential: &Credential<'_>,
) -> Result<(), Failure> {
    let entry = Entry {
        server_url: credential.server_url.to_owned(),
        client_info: ClientInfo {
            client_id: credential.client_id.to_owned(),
            client_secret: credential.client_secret.map(str::to_owned),
            client_secret_expires_at: None,
        },
        tokens: credential.tokens.clone(),
    };
    write_entry_under(data_dir, name, &entry)
}

/// `opencode mcp auth` exits 0 even when it prints `Authentication failed`
/// (measured 2026-08-26), so the store is the only honest answer.
pub(crate) fn verify_authenticated(name: &str) -> Result<(), Failure> {
    verify_authenticated_under(&fs::data_dir()?, name)
}

pub(crate) fn verify_authenticated_under(data_dir: &Utf8Path, name: &str) -> Result<(), Failure> {
    if has_tokens_under(data_dir, name)? {
        return Ok(());
    }
    Err(Failure::failed(
        "mcp.auth_not_verified",
        format!(
            "`opencode mcp auth {name}` exited 0, but no tokens for \
             `{name}` were found in OpenCode's own credential store"
        ),
    )
    .expected("a `tokens` entry for this server in OpenCode's mcp-auth.json")
    .actual(
        "no tokens present - `opencode mcp auth` exits 0 even when it prints \
         `Authentication failed` (measured 2026-08-26)",
    )
    .fix(FixAction::safe(
        "mcp.retry_auth",
        "Read the command's output above, then run `ivar mcp auth` again.",
    )))
}

#[cfg(test)]
#[path = "../../../tests/unit/providers/opencode/auth.rs"]
mod tests;
