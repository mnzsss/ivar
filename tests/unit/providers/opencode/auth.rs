#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use std::collections::BTreeMap;

use super::*;
use crate::infra::oauth::Tokens;
use crate::infra::{fs, json};
use crate::providers::Credential;
use crate::test_support::utf8_temp_dir;

// ---------------------------------------------------------------------------
// Helper: build an Entry without exposing secrets in test code.
// ---------------------------------------------------------------------------

fn test_entry(server_url: &str) -> Entry {
    Entry {
        server_url: server_url.to_owned(),
        client_info: ClientInfo {
            client_id: "test-client-id".to_owned(),
            client_secret: None,
            client_secret_expires_at: None,
        },
        tokens: Tokens {
            access_token: "test-access-token".to_owned(),
            refresh_token: None,
            expires_at: None,
            scope: None,
        },
    }
}

fn credential_fixture() -> (Tokens, String, String) {
    (
        Tokens {
            access_token: "test-access".to_owned(),
            refresh_token: Some("test-refresh".to_owned()),
            expires_at: Some(1_800_000_000.0),
            scope: None,
        },
        "https://mcp.figma.com/mcp".to_owned(),
        "test-client-id".to_owned(),
    )
}

// ---------------------------------------------------------------------------
// has_tokens (preserved from Wave 1)
// ---------------------------------------------------------------------------

#[test]
fn absent_file_has_no_tokens() {
    let (_dir, root) = utf8_temp_dir();

    assert!(!has_tokens_under(&root, "acme-figma").unwrap());
}

#[test]
fn a_server_with_a_tokens_object_is_authenticated() {
    let (_dir, root) = utf8_temp_dir();
    let path = store_path(&root);
    json::write_canonical(
        &path,
        &serde_json::json!({
            "acme-figma": { "tokens": { "accessToken": "abc", "refreshToken": "def" } },
        }),
    )
    .unwrap();

    assert!(has_tokens_under(&root, "acme-figma").unwrap());
}

#[test]
fn a_server_present_but_without_tokens_is_not_authenticated() {
    let (_dir, root) = utf8_temp_dir();
    let path = store_path(&root);
    json::write_canonical(
        &path,
        &serde_json::json!({
            "acme-figma": {},
        }),
    )
    .unwrap();

    assert!(!has_tokens_under(&root, "acme-figma").unwrap());
}

#[test]
fn a_different_servers_tokens_do_not_leak_onto_this_one() {
    let (_dir, root) = utf8_temp_dir();
    let path = store_path(&root);
    json::write_canonical(
        &path,
        &serde_json::json!({
            "acme-linear": { "tokens": { "accessToken": "abc" } },
        }),
    )
    .unwrap();

    assert!(!has_tokens_under(&root, "acme-figma").unwrap());
}

#[test]
fn auth_path_under_joins_the_opencode_store_filename() {
    let root = camino::Utf8PathBuf::from("/data");
    assert_eq!(
        store_path(&root),
        camino::Utf8PathBuf::from("/data/opencode/mcp-auth.json")
    );
}

// ---------------------------------------------------------------------------
// has_tokens: tokens with null value is not authenticated
// ---------------------------------------------------------------------------

#[test]
fn a_null_tokens_value_is_not_authenticated() {
    let (_dir, root) = utf8_temp_dir();
    let path = store_path(&root);
    json::write_canonical(
        &path,
        &serde_json::json!({
            "acme-figma": { "tokens": null },
        }),
    )
    .unwrap();

    assert!(!has_tokens_under(&root, "acme-figma").unwrap());
}

#[test]
fn a_non_object_tokens_value_is_not_authenticated() {
    let (_dir, root) = utf8_temp_dir();
    let path = store_path(&root);
    json::write_canonical(
        &path,
        &serde_json::json!({
            "acme-figma": { "tokens": "not-a-token-object" },
        }),
    )
    .unwrap();

    assert!(!has_tokens_under(&root, "acme-figma").unwrap());
}

// ---------------------------------------------------------------------------
// has_entry_under
// ---------------------------------------------------------------------------

#[test]
fn has_entry_returns_false_when_store_is_absent() {
    let (_dir, root) = utf8_temp_dir();
    assert!(!has_entry_under(&root, "acme-figma").unwrap());
}

#[test]
fn has_entry_returns_true_when_entry_has_only_code_verifier() {
    let (_dir, root) = utf8_temp_dir();
    let path = store_path(&root);
    json::write_canonical(
        &path,
        &serde_json::json!({
            "acme-figma": { "codeVerifier": "abc" },
        }),
    )
    .unwrap();

    assert!(has_entry_under(&root, "acme-figma").unwrap());
}

#[test]
fn has_entry_returns_true_for_empty_object_entry() {
    let (_dir, root) = utf8_temp_dir();
    let path = store_path(&root);
    json::write_canonical(
        &path,
        &serde_json::json!({
            "acme-figma": {},
        }),
    )
    .unwrap();

    assert!(has_entry_under(&root, "acme-figma").unwrap());
}

#[test]
fn has_entry_returns_true_for_entry_with_client_info_only() {
    let (_dir, root) = utf8_temp_dir();
    let path = store_path(&root);
    json::write_canonical(
        &path,
        &serde_json::json!({
            "acme-figma": {
                "clientInfo": { "clientId": "xyz" }
            },
        }),
    )
    .unwrap();

    assert!(has_entry_under(&root, "acme-figma").unwrap());
}

#[test]
fn has_entry_returns_true_for_full_entry() {
    let (_dir, root) = utf8_temp_dir();
    let path = store_path(&root);
    json::write_canonical(
        &path,
        &serde_json::json!({
            "acme-figma": {
                "tokens": { "accessToken": "abc" },
                "clientInfo": { "clientId": "xyz" }
            },
        }),
    )
    .unwrap();

    assert!(has_entry_under(&root, "acme-figma").unwrap());
}

#[test]
fn has_entry_returns_false_for_different_name() {
    let (_dir, root) = utf8_temp_dir();
    let path = store_path(&root);
    json::write_canonical(
        &path,
        &serde_json::json!({
            "acme-linear": {},
        }),
    )
    .unwrap();

    assert!(!has_entry_under(&root, "acme-figma").unwrap());
}

// ---------------------------------------------------------------------------
// write_entry_under
// ---------------------------------------------------------------------------

#[test]
fn write_entry_under_creates_file_with_correct_camel_case_shape() {
    let (_dir, root) = utf8_temp_dir();
    let entry = test_entry("https://mcp.figma.com");
    write_entry_under(&root, "acme-figma", &entry).unwrap();

    let path = store_path(&root);
    let content = fs::read_text(&path).unwrap().unwrap();

    // Verify the JSON contains camelCase keys
    assert!(
        content.contains("\"accessToken\""),
        "expected camelCase accessToken, got: {content}"
    );
    assert!(
        content.contains("\"serverUrl\""),
        "expected camelCase serverUrl, got: {content}"
    );
    assert!(
        content.contains("\"clientId\""),
        "expected camelCase clientId, got: {content}"
    );
    assert!(
        content.contains("https://mcp.figma.com"),
        "expected server URL, got: {content}"
    );
    assert!(
        content.contains("test-access-token"),
        "expected access token value, got: {content}"
    );
}

#[test]
fn write_entry_under_preserves_unrelated_entry() {
    let (_dir, root) = utf8_temp_dir();

    // Pre-existing entry for a different server.
    let path = store_path(&root);
    json::write_canonical(
        &path,
        &serde_json::json!({
            "acme-linear": {
                "tokens": { "accessToken": "existing-token" }
            },
        }),
    )
    .unwrap();

    let entry = test_entry("https://mcp.figma.com");
    write_entry_under(&root, "acme-figma", &entry).unwrap();

    // The unrelated entry must be preserved exactly.
    let after_map: BTreeMap<String, serde_json::Value> =
        serde_json::from_slice(&fs::read_bytes(&path).unwrap().unwrap()).unwrap();

    let linear = after_map.get("acme-linear").unwrap();
    let tokens = linear.get("tokens").unwrap();
    assert_eq!(
        tokens.get("accessToken").and_then(|v| v.as_str()),
        Some("existing-token"),
        "the unrelated entry's tokens must be preserved"
    );

    // The new entry must be present.
    assert!(after_map.contains_key("acme-figma"));
}

#[cfg(unix)]
#[test]
fn write_entry_under_file_has_0600_mode_on_unix() {
    let (_dir, root) = utf8_temp_dir();
    let entry = test_entry("https://mcp.figma.com");
    write_entry_under(&root, "acme-figma", &entry).unwrap();

    let path = store_path(&root);
    let mode = std::fs::metadata(path.as_std_path())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "store file must have mode 0600");
}

// ---------------------------------------------------------------------------
// Conflict detection (R-CONFLICT, C-NO-OVERWRITE)
// ---------------------------------------------------------------------------

#[test]
fn write_entry_under_aborts_on_same_name_conflict() {
    let (_dir, root) = utf8_temp_dir();
    let path = store_path(&root);
    json::write_canonical(
        &path,
        &serde_json::json!({
            "acme-figma": { "codeVerifier": "existing" },
        }),
    )
    .unwrap();
    let original_bytes = fs::read_bytes(&path).unwrap().unwrap();

    let entry = test_entry("https://mcp.figma.com");
    let result = write_entry_under(&root, "acme-figma", &entry);

    let err = result.unwrap_err();
    assert_eq!(err.code, "opencode_auth.conflict");
    assert!(
        err.what.contains("acme-figma"),
        "error must name the conflicting server: {}",
        err.what
    );

    // The file must be unchanged.
    let after_bytes = fs::read_bytes(&path).unwrap().unwrap();
    assert_eq!(
        original_bytes, after_bytes,
        "conflict must not modify the store"
    );
}

// ---------------------------------------------------------------------------
// Invalid JSON handling
// ---------------------------------------------------------------------------

#[test]
fn write_entry_under_returns_error_for_invalid_json_and_leaves_bytes_unchanged() {
    let (_dir, root) = utf8_temp_dir();
    let path = store_path(&root);
    if let Some(parent) = path.parent() {
        fs::ensure_dir(parent).unwrap();
    }
    fs::write_text(&path, "NOT JSON!!!").unwrap();
    let original_bytes = fs::read_bytes(&path).unwrap().unwrap();

    let entry = test_entry("https://mcp.figma.com");
    let result = write_entry_under(&root, "acme-figma", &entry);

    assert!(result.is_err(), "invalid JSON should produce an error");

    let after_bytes = fs::read_bytes(&path).unwrap().unwrap();
    assert_eq!(
        original_bytes, after_bytes,
        "invalid JSON must not modify the file"
    );
}

#[test]
fn has_entry_under_returns_error_for_invalid_json() {
    let (_dir, root) = utf8_temp_dir();
    let path = store_path(&root);
    if let Some(parent) = path.parent() {
        fs::ensure_dir(parent).unwrap();
    }
    fs::write_text(&path, "{bad json").unwrap();

    let result = has_entry_under(&root, "acme-figma");
    assert!(result.is_err(), "invalid JSON should produce an error");
}

// ---------------------------------------------------------------------------
// read_map_under: absent store returns empty map
// ---------------------------------------------------------------------------

#[test]
fn read_map_under_returns_empty_for_absent_file() {
    let (_dir, root) = utf8_temp_dir();
    let map = read_map_under(&root).unwrap();
    assert!(map.is_empty());
}

// ---------------------------------------------------------------------------
// Task 09 Additions
// ---------------------------------------------------------------------------

#[test]
fn install_credentials_writes_the_opencode_entry_shape() {
    let (_guard, root) = crate::test_support::hall_root();
    let (tokens, url, client_id) = credential_fixture();
    let credential = Credential {
        server_url: &url,
        client_id: &client_id,
        client_secret: Some("test-secret"),
        tokens: &tokens,
    };

    install_credentials_under(&root, "acme-figma", &credential).unwrap();

    let raw = crate::infra::fs::read_text(&store_path(&root))
        .unwrap()
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        parsed["acme-figma"]["serverUrl"],
        "https://mcp.figma.com/mcp"
    );
    assert_eq!(
        parsed["acme-figma"]["clientInfo"]["clientId"],
        "test-client-id"
    );
    assert_eq!(parsed["acme-figma"]["tokens"]["accessToken"], "test-access");
    assert!(has_tokens_under(&root, "acme-figma").unwrap());
}

#[test]
fn install_credentials_refuses_to_overwrite_an_existing_entry() {
    let (_guard, root) = crate::test_support::hall_root();
    let (tokens, url, client_id) = credential_fixture();
    let credential = Credential {
        server_url: &url,
        client_id: &client_id,
        client_secret: None,
        tokens: &tokens,
    };

    install_credentials_under(&root, "acme-figma", &credential).unwrap();
    let err = install_credentials_under(&root, "acme-figma", &credential).unwrap_err();

    assert_eq!(err.code, "opencode_auth.conflict");
}

/// The diagnostic this returns is the one `dispatch.rs` returned before the
/// cutover, character for character. It is the only honest answer OpenCode
/// gives, and rewording it would be a regression a passing test would hide.
#[test]
fn verify_authenticated_preserves_the_existing_diagnostic() {
    let (_guard, root) = crate::test_support::hall_root();

    let err = verify_authenticated_under(&root, "acme-figma").unwrap_err();

    assert_eq!(err.code, "mcp.auth_not_verified");
    assert!(
        err.what.contains("`opencode mcp auth acme-figma` exited 0"),
        "what was: {}",
        err.what
    );
    assert_eq!(
        err.expected.as_deref(),
        Some("a `tokens` entry for this server in OpenCode's mcp-auth.json")
    );
    assert!(err.fix_actions.iter().any(|f| f.code == "mcp.retry_auth"));
}

/// A redacted `Debug` is load-bearing: this struct carries a live access
/// token, and one `{:?}` in a log would leak it.
#[test]
fn credential_debug_redacts_every_secret() {
    let (tokens, url, client_id) = credential_fixture();
    let credential = Credential {
        server_url: &url,
        client_id: &client_id,
        client_secret: Some("test-secret"),
        tokens: &tokens,
    };

    let rendered = format!("{credential:?}");
    assert!(rendered.contains("https://mcp.figma.com/mcp"));
    assert!(!rendered.contains("test-access"));
    assert!(!rendered.contains("test-refresh"));
    assert!(!rendered.contains("test-secret"));
    assert!(!rendered.contains("test-client-id"));
}
