//! MCP config materialisation: the hall's MCP server definitions written to
//! the per-provider config file at the hall root, one JSON key at a time.
//!
//! The "file belongs to the user" rule from the parent module applies here
//! with a JSON key standing in for the marker pair: `.mcp.json` is owned
//! wholesale, while `opencode.json`'s `mcp` key is merged and every other key
//! the user wrote survives. The Claude/OpenCode spelling translation lives
//! here.
//!
//! # `oauth`: OpenCode only, and never a secret value
//!
//! A server whose manifest entry carries `McpServerDef.oauth` gets an
//! `oauth` object in its OpenCode entry: `clientId` and `redirectUri`
//! literal, `clientSecret` as the `{env:NAME}` reference the manifest names
//! — never the secret itself, since `McpOauth` has no field that could hold
//! one. Claude Code's branch never emits this key at all: Claude Code is on
//! the remote host's allowlist and needs no pre-registration.

/// Materialise `provider`'s MCP config at `path` from `servers`.
///
/// This is the JSON half of the instruction-file block: idempotent by
/// comparison (nothing is written when the bytes already match), and respectful
/// of the user's own bytes — for a provider whose config file carries more than
/// MCP (OpenCode's `opencode.json`), only the provider's `mcp` key is replaced.
/// A file that exists but cannot be parsed as a JSON object is refused, never
/// clobbered.
use camino::Utf8Path;

use crate::domain::mcp::McpServerDef;
use crate::domain::name::HallName;
use crate::domain::provider::Provider;
use crate::infra::{fs, json};

use crate::providers;

use super::{Change, Error};
/// The redirect URI a pre-registered OpenCode OAuth client declares, and the
/// one `opencode.json`'s `oauth.redirectUri` must repeat so OpenCode listens
/// where the registration told the server it would.
///
/// The path (`/callback`) is the remote host's requirement, not OpenCode's
/// own default (`/mcp/oauth/callback`) — Figma's registration endpoint
/// returns `400 invalid_redirect_uri` for anything else (measured
/// 2026-08-26; see `plans/ivar-mcp-auth/analysis.md`). The port (`19876`) is
/// OpenCode's own default callback port; setting `oauth.redirectUri`
/// overrides both OpenCode's callback server and its authorize request, so
/// declaring it here is what reconciles the two rather than leaving them to
/// disagree.
pub const OAUTH_REDIRECT_URI: &str = "http://127.0.0.1:19876/callback";

pub fn materialise_mcp(
    path: &Utf8Path,
    provider: Provider,
    servers: &[McpServerDef],
    hall: &HallName,
) -> Result<Change, Error> {
    let servers_value = servers_doc(provider, servers, hall);
    let (existing, raw) = read_doc(path)?;

    let Some(mut doc) = existing else {
        return write_doc(path, &mcp_doc(provider, servers_value)).map(|_| Change::Created);
    };

    let object = doc.as_object_mut().ok_or_else(|| Error::McpNotObject {
        path: path.to_path_buf(),
    })?;
    object.insert(providers::mcp_root_key(provider).to_owned(), servers_value);
    // OpenCode's config carries a `$schema`; make sure one is there, without
    // clobbering one the user already wrote.
    if provider == Provider::OpenCode && !object.contains_key("$schema") {
        object.insert(
            "$schema".to_owned(),
            serde_json::json!("https://opencode.ai/config.json"),
        );
    }

    let rendered = json::to_canonical_string(&doc).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source,
    })?;
    if raw.as_deref() == Some(rendered.as_str()) {
        return Ok(Change::Unchanged);
    }

    write_doc(path, &doc)?;
    Ok(Change::Updated)
}

/// Take the MCP key out of the config at `path` — the JSON half of the
/// instruction-file strip.
///
/// The file is deleted only when the MCP key was its entire content (`.mcp.json`
/// with nothing but `mcpServers`); a file carrying other keys keeps them, minus
/// the MCP key. Absent file, or a file with no MCP key, is
/// [`Change::Unchanged`]. A file that cannot be parsed as a JSON object is
/// left alone — stripping a key out of something that is not an object has no
/// defined meaning, and deleting it would be the silent-overwrite bug again.
pub fn remove_mcp(path: &Utf8Path, provider: Provider) -> Result<Change, Error> {
    let (existing, _) = read_doc(path)?;
    let Some(mut doc) = existing else {
        return Ok(Change::Unchanged);
    };

    let Some(object) = doc.as_object_mut() else {
        return Ok(Change::Unchanged);
    };
    if object.remove(providers::mcp_root_key(provider)).is_none() {
        return Ok(Change::Unchanged);
    }

    if object.is_empty() {
        fs::remove_file(path).map_err(|source| Error::Mcp {
            path: path.to_path_buf(),
            source: json::Error::Fs(source),
        })?;
        return Ok(Change::Removed);
    }

    write_doc(path, &doc)?;
    Ok(Change::Removed)
}

/// The full document `ivar` wants for `provider`: its `mcp` key holding
/// `servers`, plus OpenCode's `$schema`. Used only when the file is absent —
/// an existing file is merged key-by-key instead ([`materialise_mcp`]).
fn mcp_doc(provider: Provider, servers: serde_json::Value) -> serde_json::Value {
    let mut root = serde_json::Map::new();
    if provider == Provider::OpenCode {
        root.insert(
            "$schema".to_owned(),
            serde_json::json!("https://opencode.ai/config.json"),
        );
    }
    root.insert(providers::mcp_root_key(provider).to_owned(), servers);
    serde_json::Value::Object(root)
}

/// The `mcp` value itself: one entry per server, keyed by its hall-qualified
/// name — the provider boundary, where two halls' servers must stay distinct.
fn servers_doc(provider: Provider, servers: &[McpServerDef], hall: &HallName) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for server in servers {
        map.insert(
            server.materialised_name(hall),
            providers::mcp_server_doc(provider, &server.materialised_name(hall), server),
        );
    }
    serde_json::Value::Object(map)
}

/// Read `path` as JSON, returning the parsed document and its raw bytes.
///
/// `Ok((None, None))` when the file is absent. A file that exists but is not
/// valid JSON is an error — never a silent clobber of user config.
fn read_doc(path: &Utf8Path) -> Result<(Option<serde_json::Value>, Option<String>), Error> {
    let Some(text) = fs::read_text(path).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source: json::Error::Fs(source),
    })?
    else {
        return Ok((None, None));
    };
    let value = serde_json::from_str(&text).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source: json::Error::Parse {
            path: path.to_path_buf(),
            source,
        },
    })?;
    Ok((Some(value), Some(text)))
}

/// Write `doc` to `path` in the canonical byte format.
fn write_doc(path: &Utf8Path, doc: &serde_json::Value) -> Result<(), Error> {
    json::write_canonical(path, doc).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source,
    })
}
