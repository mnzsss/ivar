#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::test_support::utf8_temp_dir;

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
            "acme-figma": { "tokens": { "access": "abc", "refresh": "def" } },
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
            "acme-linear": { "tokens": { "access": "abc" } },
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
