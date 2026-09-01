#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::mcp::McpServerDef;
use crate::domain::name::HallName;
use crate::domain::provider::Provider;
use crate::infra::fs;
use crate::infra::http_callback::OAUTH_REDIRECT_URI;
use crate::test_support::utf8_temp_dir;

/// The neutral fixture hall these tests materialise servers under.
fn hall() -> HallName {
    HallName::new("acme").unwrap()
}

// -- materialise_mcp: Claude Code ----------------------------------------

/// The v1 case the sync step starts from: no servers declared, and still a
/// valid — empty — config at the hall root, so walk-up discovery finds it.
#[test]
fn an_empty_server_list_materialises_a_valid_empty_config() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join(".mcp.json");

    assert_eq!(
        materialise_mcp(&path, Provider::ClaudeCode, &[], &hall()).unwrap(),
        Change::Created
    );

    let expected = json::to_canonical_string(&serde_json::json!({ "mcpServers": {} })).unwrap();
    assert_eq!(fs::read_text(&path).unwrap().unwrap(), expected);
}

#[test]
fn a_local_server_is_serialised_with_command_args_and_env() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join(".mcp.json");
    let servers = vec![
        McpServerDef::new("docs", "local")
            .command("npx")
            .args(vec!["-y".to_owned(), "@acme/docs-mcp".to_owned()])
            .env(std::collections::BTreeMap::from([(
                "TOKEN".to_owned(),
                "{env:TOKEN}".to_owned(),
            )])),
    ];

    materialise_mcp(&path, Provider::ClaudeCode, &servers, &hall()).unwrap();

    let expected = json::to_canonical_string(&serde_json::json!({
        "mcpServers": {
            "acme-docs": {
                "type": "stdio",
                "command": "npx",
                "args": ["-y", "@acme/docs-mcp"],
                "env": { "TOKEN": "{env:TOKEN}" },
            }
        }
    }))
    .unwrap();
    assert_eq!(fs::read_text(&path).unwrap().unwrap(), expected);
}

/// `ivar sync` runs after every `git pull`; a second run must touch nothing.
#[test]
fn materialising_mcp_twice_is_unchanged_and_does_not_rewrite() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join(".mcp.json");
    let servers = vec![McpServerDef::new("docs", "local").command("npx")];

    assert_eq!(
        materialise_mcp(&path, Provider::ClaudeCode, &servers, &hall()).unwrap(),
        Change::Created
    );
    let after_first = fs::read_bytes(&path).unwrap().unwrap();

    assert_eq!(
        materialise_mcp(&path, Provider::ClaudeCode, &servers, &hall()).unwrap(),
        Change::Unchanged
    );
    assert_eq!(fs::read_bytes(&path).unwrap().unwrap(), after_first);
}

#[test]
fn changed_servers_rewrite_the_config() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join(".mcp.json");
    materialise_mcp(&path, Provider::ClaudeCode, &[], &hall()).unwrap();

    let with_server = vec![McpServerDef::new("docs", "local").command("npx")];
    assert_eq!(
        materialise_mcp(&path, Provider::ClaudeCode, &with_server, &hall()).unwrap(),
        Change::Updated
    );

    let on_disk = fs::read_text(&path).unwrap().unwrap();
    assert!(on_disk.contains("\"acme-docs\""), "was: {on_disk}");
}

// -- materialise_mcp: OpenCode -------------------------------------------

#[test]
fn opencode_materialises_the_schema_key_next_to_the_mcp_key() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("opencode.json");

    materialise_mcp(&path, Provider::OpenCode, &[], &hall()).unwrap();

    let expected = json::to_canonical_string(&serde_json::json!({
        "$schema": "https://opencode.ai/config.json",
        "mcp": {},
    }))
    .unwrap();
    assert_eq!(fs::read_text(&path).unwrap().unwrap(), expected);
}

/// The same definition, spelled the way OpenCode reads it: `stdio` →
/// `local`, one `command` array, `environment` for the env map.
#[test]
fn opencode_translates_a_stdio_definition_into_its_own_spelling() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("opencode.json");
    let servers = vec![
        McpServerDef::new("docs", "local")
            .command("npx")
            .args(vec!["-y".to_owned(), "@acme/docs-mcp".to_owned()])
            .env(std::collections::BTreeMap::from([(
                "TOKEN".to_owned(),
                "{env:TOKEN}".to_owned(),
            )])),
    ];


    materialise_mcp(&path, Provider::OpenCode, &servers, &hall()).unwrap();

    let expected = json::to_canonical_string(&serde_json::json!({
        "$schema": "https://opencode.ai/config.json",
        "mcp": {
            "acme-docs": {
                "type": "local",
                "command": ["npx", "-y", "@acme/docs-mcp"],
                "environment": { "TOKEN": "{env:TOKEN}" },
            }
        }
    }))
    .unwrap();
    assert_eq!(fs::read_text(&path).unwrap().unwrap(), expected);
}

/// A server carrying `oauth` gets an `oauth` object with `clientId` literal,
/// `clientSecret` as the `{env:NAME}` reference the manifest names — never a
/// value — and the derived `redirectUri`.
#[test]
fn opencode_emits_the_oauth_block_for_a_server_that_carries_one() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("opencode.json");
    let servers = vec![
        McpServerDef::new("figma", "http")
            .url("https://mcp.figma.com/mcp")
            .oauth(crate::domain::mcp::McpOauth::new(
                "client-123",
                "IVAR_MCP_ACME_FIGMA_SECRET",
            )),
    ];

    materialise_mcp(&path, Provider::OpenCode, &servers, &hall()).unwrap();

    let expected = json::to_canonical_string(&serde_json::json!({
        "$schema": "https://opencode.ai/config.json",
        "mcp": {
            "acme-figma": {
                "type": "remote",
                "url": "https://mcp.figma.com/mcp",
                "oauth": {
                    "clientId": "client-123",
                    "clientSecret": "{env:IVAR_MCP_ACME_FIGMA_SECRET}",
                    "redirectUri": OAUTH_REDIRECT_URI,
                },
            }
        }
    }))
    .unwrap();
    assert_eq!(fs::read_text(&path).unwrap().unwrap(), expected);
}

/// Claude Code is on the remote host's allowlist and needs no
/// pre-registration — its branch must never emit the `oauth` key even for a
/// server whose manifest entry carries one.
#[test]
fn claude_code_never_emits_the_oauth_block() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join(".mcp.json");
    let servers = vec![
        McpServerDef::new("figma", "http")
            .url("https://mcp.figma.com/mcp")
            .oauth(crate::domain::mcp::McpOauth::new(
                "client-123",
                "IVAR_MCP_ACME_FIGMA_SECRET",
            )),
    ];

    materialise_mcp(&path, Provider::ClaudeCode, &servers, &hall()).unwrap();

    let on_disk = fs::read_text(&path).unwrap().unwrap();
    assert!(
        !on_disk.contains("oauth"),
        "Claude Code must never emit an oauth block: {on_disk}"
    );
}

#[test]
fn opencode_spells_a_remote_definition_with_type_remote_and_a_url() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("opencode.json");
    let servers = vec![McpServerDef::new("sentry", "http").url("https://mcp.example.com/mcp")];

    materialise_mcp(&path, Provider::OpenCode, &servers, &hall()).unwrap();

    let on_disk = fs::read_text(&path).unwrap().unwrap();
    assert!(on_disk.contains("\"type\": \"remote\""), "was: {on_disk}");
    assert!(
        on_disk.contains("\"url\": \"https://mcp.example.com/mcp\""),
        "was: {on_disk}"
    );
}

/// `opencode.json` is OpenCode's *general* config. The user's other keys
/// must survive a sync that replaces the `mcp` key.
#[test]
fn an_existing_opencode_config_keeps_its_other_keys() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("opencode.json");
    fs::write_text(
        &path,
        &json::to_canonical_string(&serde_json::json!({
            "$schema": "https://opencode.ai/config.json",
            "model": "anthropic/claude-sonnet-4-5",
            "mcp": { "stale": { "type": "local", "command": ["old"] } },
        }))
        .unwrap(),
    )
    .unwrap();

    let servers = vec![McpServerDef::new("docs", "local").command("npx")];
    assert_eq!(
        materialise_mcp(&path, Provider::OpenCode, &servers, &hall()).unwrap(),
        Change::Updated
    );

    let on_disk = fs::read_text(&path).unwrap().unwrap();
    assert!(
        on_disk.contains("claude-sonnet-4-5"),
        "the user's model must survive: {on_disk}"
    );
    assert!(
        !on_disk.contains("\"stale\""),
        "the mcp key must be replaced: {on_disk}"
    );
    assert!(
        on_disk.contains("\"acme-docs\""),
        "the manifest's servers must land: {on_disk}"
    );

    // And the next sync touches nothing.
    assert_eq!(
        materialise_mcp(&path, Provider::OpenCode, &servers, &hall()).unwrap(),
        Change::Unchanged
    );
}

// -- materialise_mcp: never clobber what cannot be parsed ----------------

#[test]
fn a_config_that_is_not_an_object_is_refused_not_clobbered() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("opencode.json");
    fs::write_text(&path, "[1, 2, 3]").unwrap();

    let error = materialise_mcp(&path, Provider::OpenCode, &[], &hall())
        .expect_err("an array cannot take an mcp key");

    let failure: Failure = error.into();
    assert_eq!(failure.code, "harness.mcp_not_an_object");
    assert_eq!(
        fs::read_text(&path).unwrap().unwrap(),
        "[1, 2, 3]",
        "the file must be left exactly as it was"
    );
}

#[test]
fn a_config_that_is_not_valid_json_is_refused_not_clobbered() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join(".mcp.json");
    fs::write_text(&path, "{ not json").unwrap();

    let error =
        materialise_mcp(&path, Provider::ClaudeCode, &[], &hall()).expect_err("unparseable");

    let failure: Failure = error.into();
    assert!(
        failure
            .fix_actions
            .iter()
            .any(|fix| fix.code == "harness.check_mcp_config"),
        "expected the mcp fix action, got {:?}",
        failure.fix_actions
    );
    assert_eq!(
        fs::read_text(&path).unwrap().unwrap(),
        "{ not json",
        "the file must be left exactly as it was"
    );
}

// -- remove_mcp ----------------------------------------------------------

#[test]
fn removing_an_exclusively_mcp_file_deletes_it() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join(".mcp.json");
    materialise_mcp(&path, Provider::ClaudeCode, &[], &hall()).unwrap();

    assert_eq!(
        remove_mcp(&path, Provider::ClaudeCode).unwrap(),
        Change::Removed
    );
    assert!(!fs::exists(&path).unwrap());
}

#[test]
fn removing_the_mcp_key_keeps_a_file_with_other_keys() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("opencode.json");
    fs::write_text(
        &path,
        &json::to_canonical_string(&serde_json::json!({
            "$schema": "https://opencode.ai/config.json",
            "model": "anthropic/claude-sonnet-4-5",
            "mcp": {},
        }))
        .unwrap(),
    )
    .unwrap();

    assert_eq!(
        remove_mcp(&path, Provider::OpenCode).unwrap(),
        Change::Removed
    );

    let on_disk = fs::read_text(&path).unwrap().unwrap();
    assert!(
        on_disk.contains("claude-sonnet-4-5"),
        "the user's keys must survive: {on_disk}"
    );
    assert!(
        !on_disk.contains("\"mcp\""),
        "the mcp key must be gone: {on_disk}"
    );
}

#[test]
fn removing_mcp_when_there_is_nothing_to_remove_is_unchanged() {
    let (_guard, dir) = utf8_temp_dir();

    let absent = dir.join(".mcp.json");
    assert_eq!(
        remove_mcp(&absent, Provider::ClaudeCode).unwrap(),
        Change::Unchanged
    );

    let without_mcp = dir.join("opencode.json");
    fs::write_text(&without_mcp, "{\"model\": \"x\"}").unwrap();
    assert_eq!(
        remove_mcp(&without_mcp, Provider::OpenCode).unwrap(),
        Change::Unchanged
    );
    assert_eq!(
        fs::read_text(&without_mcp).unwrap().unwrap(),
        "{\"model\": \"x\"}",
        "a file with no mcp key is not rewritten"
    );
}
