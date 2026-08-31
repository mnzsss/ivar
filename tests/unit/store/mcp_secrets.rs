#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::infra::fs;
use crate::test_support::utf8_temp_dir;

#[test]
fn missing_file_reads_as_empty_store() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let secrets = McpSecrets::read(&layout).unwrap();

    assert_eq!(secrets.get("ANY_KEY"), None);
    assert!(!secrets.contains_key("ANY_KEY"));
}

#[test]
fn first_write_creates_mcp_secrets_env_file() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);

    let change =
        McpSecrets::set_and_write(&layout, "IVAR_MCP_FIGMA_SECRET", "figma-secret-value-123")
            .unwrap();
    assert_eq!(change, Change::Created);

    let mcp_env = layout.mcp_secrets_env();
    assert!(fs::exists(&mcp_env).unwrap());

    let secrets = McpSecrets::read(&layout).unwrap();
    assert_eq!(
        secrets.get("IVAR_MCP_FIGMA_SECRET"),
        Some("figma-secret-value-123")
    );
}

#[test]
#[cfg(unix)]
fn resulting_file_is_0600_on_unix() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);

    McpSecrets::set_and_write(&layout, "KEY1", "val1").unwrap();
    let mode = fs::stat(&layout.mcp_secrets_env())
        .unwrap()
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn setting_a_second_key_preserves_the_first() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);

    assert_eq!(
        McpSecrets::set_and_write(&layout, "KEY_A", "val_a").unwrap(),
        Change::Created
    );
    assert_eq!(
        McpSecrets::set_and_write(&layout, "KEY_B", "val_b").unwrap(),
        Change::Created
    );

    let secrets = McpSecrets::read(&layout).unwrap();
    assert_eq!(secrets.get("KEY_A"), Some("val_a"));
    assert_eq!(secrets.get("KEY_B"), Some("val_b"));
}

#[test]
fn updating_a_key_replaces_only_that_value() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);

    McpSecrets::set_and_write(&layout, "KEY_A", "val_a").unwrap();
    McpSecrets::set_and_write(&layout, "KEY_B", "val_b").unwrap();

    let change = McpSecrets::set_and_write(&layout, "KEY_A", "val_a_updated").unwrap();
    assert_eq!(change, Change::Updated);

    let unchanged = McpSecrets::set_and_write(&layout, "KEY_A", "val_a_updated").unwrap();
    assert_eq!(unchanged, Change::Unchanged);

    let secrets = McpSecrets::read(&layout).unwrap();
    assert_eq!(secrets.get("KEY_A"), Some("val_a_updated"));
    assert_eq!(secrets.get("KEY_B"), Some("val_b"));
}

#[test]
fn deterministic_ordering_and_trailing_newline() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);

    McpSecrets::set_and_write(&layout, "ZEBRA", "z").unwrap();
    McpSecrets::set_and_write(&layout, "ALPHA", "a").unwrap();
    McpSecrets::set_and_write(&layout, "BETA", "b").unwrap();

    let raw = fs::read_text(&layout.mcp_secrets_env()).unwrap().unwrap();
    assert_eq!(raw, "ALPHA=\"a\"\nBETA=\"b\"\nZEBRA=\"z\"\n");
}

#[test]
fn round_trips_spaces_quotes_backslashes_equals_unicode_and_newlines() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);

    let complex_val = "hello \"world\" \\ with / = symbols\nsecond line\r\nand 🦀 emoji 'single'";
    McpSecrets::set_and_write(&layout, "COMPLEX_SECRET", complex_val).unwrap();

    let secrets = McpSecrets::read(&layout).unwrap();
    assert_eq!(secrets.get("COMPLEX_SECRET"), Some(complex_val));
}

#[test]
fn malformed_syntax_fails_without_leaking_raw_line() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let path = layout.mcp_secrets_env();

    // Write malformed line missing '='
    fs::write_sensitive_atomic(&path, b"THIS_IS_VERY_SECRET_RAW_LINE_NO_EQUALS\n").unwrap();
    let failure = McpSecrets::read(&layout).unwrap_err();
    assert_eq!(failure.code, "store.mcp_secrets_malformed");
    assert!(
        !failure
            .what
            .contains("THIS_IS_VERY_SECRET_RAW_LINE_NO_EQUALS")
    );
    assert!(failure.what.contains("malformed line (missing `=`)"));

    // Write malformed line with invalid key
    fs::write_sensitive_atomic(&path, b"123_INVALID=super_secret\n").unwrap();
    let failure = McpSecrets::read(&layout).unwrap_err();
    assert_eq!(failure.code, "store.mcp_secrets_invalid_key");
    assert!(!failure.what.contains("super_secret"));

    // Write malformed line with unterminated quote
    fs::write_sensitive_atomic(&path, b"VALID_KEY=\"unterminated_secret\n").unwrap();
    let failure = McpSecrets::read(&layout).unwrap_err();
    assert_eq!(failure.code, "store.mcp_secrets_invalid_value");
    assert!(!failure.what.contains("unterminated_secret"));
}

#[test]
fn duplicate_key_policy_fails_deterministically() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let path = layout.mcp_secrets_env();

    fs::write_sensitive_atomic(&path, b"KEY_DUP=\"first\"\nKEY_DUP=\"second\"\n").unwrap();
    let failure = McpSecrets::read(&layout).unwrap_err();
    assert_eq!(failure.code, "store.mcp_secrets_duplicate_key");
    assert!(!failure.what.contains("first"));
    assert!(!failure.what.contains("second"));
    assert!(failure.what.contains("duplicate key `KEY_DUP`"));
}

#[test]
fn debug_formatting_redacts_secret_values() {
    let mut secrets = McpSecrets::default();
    secrets.entries.insert(
        "SECRET_KEY_NAME".to_owned(),
        "SUPER_SECRET_VALUE".to_owned(),
    );

    let debug_str = format!("{secrets:?}");
    assert!(debug_str.contains("SECRET_KEY_NAME"));
    assert!(!debug_str.contains("SUPER_SECRET_VALUE"));
}
