#![allow(clippy::unwrap_used)]

use super::*;
use crate::domain::mcp::CredentialState;
use crate::infra::json;
use crate::test_support::utf8_temp_dir;

#[test]
fn absent_credentials_file_returns_unknown() {
    let (_dir, root) = utf8_temp_dir();
    let state = credential_state_under(
        &root,
        "acme-linear",
        "https://mcp.linear.app/mcp",
        1_700_000_000_000,
    );
    assert_eq!(state, CredentialState::Unknown);
}

#[test]
fn unparseable_credentials_file_returns_unknown() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join(".credentials.json");
    crate::infra::fs::write_text(&path, "not json").unwrap();
    let state = credential_state_under(
        &root,
        "acme-linear",
        "https://mcp.linear.app/mcp",
        1_700_000_000_000,
    );
    assert_eq!(state, CredentialState::Unknown);
}

#[test]
fn missing_entry_returns_missing() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join(".credentials.json");
    json::write_canonical(
        &path,
        &serde_json::json!({
            "mcpOAuth": {}
        }),
    )
    .unwrap();
    let state = credential_state_under(
        &root,
        "acme-linear",
        "https://mcp.linear.app/mcp",
        1_700_000_000_000,
    );
    assert_eq!(state, CredentialState::Missing);
}

#[test]
fn empty_access_token_returns_missing() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join(".credentials.json");
    json::write_canonical(
        &path,
        &serde_json::json!({
            "mcpOAuth": {
                "acme-linear|638130d5ab3558f4": {
                    "serverName": "acme-linear",
                    "serverUrl": "https://mcp.linear.app/mcp",
                    "accessToken": "",
                    "refreshToken": "refresh-token",
                    "expiresAt": 1_800_000_000_000u64
                }
            }
        }),
    )
    .unwrap();
    let state = credential_state_under(
        &root,
        "acme-linear",
        "https://mcp.linear.app/mcp",
        1_700_000_000_000,
    );
    assert_eq!(state, CredentialState::Missing);
}

#[test]
fn valid_unexpired_entry_returns_authenticated() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join(".credentials.json");
    json::write_canonical(
        &path,
        &serde_json::json!({
            "mcpOAuth": {
                "acme-linear|638130d5ab3558f4": {
                    "serverName": "acme-linear",
                    "serverUrl": "https://mcp.linear.app/mcp",
                    "accessToken": "valid-token",
                    "expiresAt": 1_800_000_000_000u64
                }
            }
        }),
    )
    .unwrap();
    let state = credential_state_under(
        &root,
        "acme-linear",
        "https://mcp.linear.app/mcp",
        1_700_000_000_000,
    );
    assert_eq!(state, CredentialState::Authenticated);
}

#[test]
fn null_expires_at_never_expires_and_returns_authenticated() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join(".credentials.json");
    json::write_canonical(
        &path,
        &serde_json::json!({
            "mcpOAuth": {
                "acme-linear|hash": {
                    "serverName": "acme-linear",
                    "serverUrl": "https://mcp.linear.app/mcp",
                    "accessToken": "valid-token",
                    "expiresAt": null
                }
            }
        }),
    )
    .unwrap();
    let state = credential_state_under(
        &root,
        "acme-linear",
        "https://mcp.linear.app/mcp",
        1_700_000_000_000,
    );
    assert_eq!(state, CredentialState::Authenticated);
}

#[test]
fn expired_entry_with_refresh_token_returns_expired() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join(".credentials.json");
    json::write_canonical(
        &path,
        &serde_json::json!({
            "mcpOAuth": {
                "acme-linear|hash": {
                    "serverName": "acme-linear",
                    "serverUrl": "https://mcp.linear.app/mcp",
                    "accessToken": "old-token",
                    "refreshToken": "refresh-token",
                    "expiresAt": 1_600_000_000_000u64
                }
            }
        }),
    )
    .unwrap();
    let state = credential_state_under(
        &root,
        "acme-linear",
        "https://mcp.linear.app/mcp",
        1_700_000_000_000,
    );
    assert_eq!(state, CredentialState::Expired);
}

#[test]
fn expired_entry_without_refresh_token_returns_missing() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join(".credentials.json");
    json::write_canonical(
        &path,
        &serde_json::json!({
            "mcpOAuth": {
                "acme-linear|hash": {
                    "serverName": "acme-linear",
                    "serverUrl": "https://mcp.linear.app/mcp",
                    "accessToken": "old-token",
                    "refreshToken": "",
                    "expiresAt": 1_600_000_000_000u64
                }
            }
        }),
    )
    .unwrap();
    let state = credential_state_under(
        &root,
        "acme-linear",
        "https://mcp.linear.app/mcp",
        1_700_000_000_000,
    );
    assert_eq!(state, CredentialState::Missing);
}

#[test]
fn best_matching_entry_is_selected_when_multiple_hashes_present() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join(".credentials.json");
    json::write_canonical(
        &path,
        &serde_json::json!({
            "mcpOAuth": {
                "acme-linear|old": {
                    "serverName": "acme-linear",
                    "serverUrl": "https://mcp.linear.app/mcp",
                    "accessToken": "old-token",
                    "refreshToken": "refresh-token",
                    "expiresAt": 1_600_000_000_000u64
                },
                "acme-linear|new": {
                    "serverName": "acme-linear",
                    "serverUrl": "https://mcp.linear.app/mcp",
                    "accessToken": "new-token",
                    "expiresAt": 1_800_000_000_000u64
                }
            }
        }),
    )
    .unwrap();
    let state = credential_state_under(
        &root,
        "acme-linear",
        "https://mcp.linear.app/mcp",
        1_700_000_000_000,
    );
    assert_eq!(state, CredentialState::Authenticated);
}
