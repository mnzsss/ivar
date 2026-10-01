use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;
use std::collections::BTreeMap;

use crate::domain::mcp::CredentialState;
use crate::error::Failure;
use crate::infra::fs;

pub(crate) const LOGIN_SUBCOMMAND: [&str; 2] = ["mcp", "login"];

/// Determine Claude Code configuration directory.
/// Uses `$CLAUDE_CONFIG_DIR` if set, otherwise `$HOME/.claude`.
pub(crate) fn config_dir() -> Result<Utf8PathBuf, Failure> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        return Ok(Utf8PathBuf::from(dir));
    }
    let home = crate::providers::user_home()?;
    Ok(home.join(".claude"))
}

#[derive(Deserialize)]
struct ClaudeCredentialsFile {
    #[serde(default, rename = "mcpOAuth")]
    mcp_oauth: BTreeMap<String, ClaudeOAuthEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeOAuthEntry {
    #[serde(default)]
    server_url: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_at: Option<u64>,
}

/// Inspect Claude Code credentials under `config_dir` for a specific server.
pub(crate) fn credential_state_under(
    config_dir: &Utf8Path,
    name: &str,
    server_url: &str,
    now_ms: u64,
) -> CredentialState {
    let path = config_dir.join(".credentials.json");
    let content = match fs::read_text(&path) {
        Ok(Some(text)) => text,
        Ok(None) => return CredentialState::Unknown,
        Err(_) => return CredentialState::Unknown,
    };

    let parsed: ClaudeCredentialsFile = match serde_json::from_str(&content) {
        Ok(file) => file,
        Err(_) => return CredentialState::Unknown,
    };

    let prefix = format!("{name}|");
    let matching: Vec<&ClaudeOAuthEntry> = parsed
        .mcp_oauth
        .iter()
        .filter(|(key, entry)| {
            key.starts_with(&prefix) && entry.server_url.as_deref() == Some(server_url)
        })
        .map(|(_, entry)| entry)
        .collect();

    if matching.is_empty() {
        return CredentialState::Missing;
    }

    // Filter to entries with non-empty accessToken
    let with_tokens: Vec<&&ClaudeOAuthEntry> = matching
        .iter()
        .filter(|e| {
            e.access_token
                .as_ref()
                .is_some_and(|t| !t.trim().is_empty())
        })
        .collect();

    if with_tokens.is_empty() {
        return CredentialState::Missing;
    }

    // Best entry: max expiresAt (null expiresAt is treated as infinite / max)
    let best = with_tokens
        .into_iter()
        .max_by_key(|e| e.expires_at.unwrap_or(u64::MAX));

    let Some(entry) = best else {
        return CredentialState::Missing;
    };

    match entry.expires_at {
        Some(exp) if exp <= now_ms => {
            if entry
                .refresh_token
                .as_ref()
                .is_some_and(|rt| !rt.trim().is_empty())
            {
                CredentialState::Expired
            } else {
                CredentialState::Missing
            }
        }
        _ => CredentialState::Authenticated,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/providers/claude_code/auth.rs"]
mod tests;
