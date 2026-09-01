#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::test_support::utf8_temp_dir;

// ---------------------------------------------------------------------------
// Helper: build an Entry without exposing secrets in test code.
// ---------------------------------------------------------------------------

fn test_entry(server_url: &str) -> Entry {
    use crate::infra::oauth::Tokens;

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
    let path = auth_path_under(&root);
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
    let path = auth_path_under(&root);
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
    let path = auth_path_under(&root);
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
        auth_path_under(&root),
        camino::Utf8PathBuf::from("/data/opencode/mcp-auth.json")
    );
}

// ---------------------------------------------------------------------------
// has_tokens: tokens with null value is not authenticated
// ---------------------------------------------------------------------------

#[test]
fn a_null_tokens_value_is_not_authenticated() {
    let (_dir, root) = utf8_temp_dir();
    let path = auth_path_under(&root);
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
    let path = auth_path_under(&root);
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
    let path = auth_path_under(&root);
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
    let path = auth_path_under(&root);
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
    let path = auth_path_under(&root);
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
    let path = auth_path_under(&root);
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
    let path = auth_path_under(&root);
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

    let path = auth_path_under(&root);
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
    let path = auth_path_under(&root);
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

    let path = auth_path_under(&root);
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
    let path = auth_path_under(&root);
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
    let path = auth_path_under(&root);
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
    let path = auth_path_under(&root);
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
