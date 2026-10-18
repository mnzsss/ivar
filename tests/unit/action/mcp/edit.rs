#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::Ctx;
use crate::action::mcp::add::{AddInput, add};
use crate::domain::mcp::McpOauth;
use crate::domain::provider::Provider;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::store::mcp_secrets::McpSecrets;
use crate::test_support::seeded_hall;

fn http(name: &str, url: &str) -> AddInput {
    AddInput {
        name: name.to_owned(),
        url: Some(url.to_owned()),
        command: None,
        args: vec![],
    }
}

fn to_url(name: &str, url: &str) -> EditInput {
    EditInput {
        name: name.to_owned(),
        url: Some(url.to_owned()),
        command: None,
        args: vec![],
    }
}

fn with_oauth(root: &camino::Utf8PathBuf, name: &str) {
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let servers = manifest
        .mcp_servers()
        .iter()
        .cloned()
        .map(|s| {
            if s.name == name {
                s.oauth(McpOauth::new("client-1", "IVAR_MCP_X_SECRET"))
            } else {
                s
            }
        })
        .collect();
    Manifest::write(&layout, &manifest.with_mcp_servers(servers).unwrap()).unwrap();
}

fn server(root: &camino::Utf8PathBuf, name: &str) -> McpServerDef {
    let manifest = Manifest::read(&Layout::at(root.clone())).unwrap().unwrap();
    manifest
        .mcp_servers()
        .iter()
        .find(|s| s.name == name)
        .unwrap()
        .clone()
}

#[test]
fn edit_replaces_the_url_drops_oauth_and_asks_for_reauth() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    add(&ctx, &http("x", "https://old.test/mcp")).unwrap();
    with_oauth(&root, "x");

    let report = edit(&ctx, &to_url("x", "https://new.test/mcp")).unwrap();

    assert!(report.value.changed);
    assert!(report.value.oauth_dropped);
    assert!(report.is_clean());
    assert_eq!(report.value.next.as_deref(), Some("ivar mcp auth x"));
    let saved = server(&root, "x");
    assert_eq!(saved.url.as_deref(), Some("https://new.test/mcp"));
    assert!(saved.oauth.is_none());
    let claude =
        std::fs::read_to_string(Layout::at(root).mcp_config(&Provider::ClaudeCode)).unwrap();
    assert!(claude.contains("https://new.test/mcp"), "{claude}");
    assert!(!claude.contains("https://old.test/mcp"), "{claude}");
}

#[test]
fn edit_can_switch_http_to_stdio() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    add(&ctx, &http("x", "https://old.test/mcp")).unwrap();

    let input = EditInput {
        name: "x".to_owned(),
        url: None,
        command: Some("npx".to_owned()),
        args: vec!["@mcp/x".to_owned()],
    };
    edit(&ctx, &input).unwrap();

    let saved = server(&root, "x");
    assert_eq!(saved.type_, "local");
    assert_eq!(saved.url, None);
    assert_eq!(saved.command.as_deref(), Some("npx"));
}

#[test]
fn an_identical_edit_keeps_oauth_and_writes_nothing() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    add(&ctx, &http("x", "https://same.test/mcp")).unwrap();
    with_oauth(&root, "x");
    let before = std::fs::read_to_string(root.join("ivar.json")).unwrap();

    let report = edit(&ctx, &to_url("x", "https://same.test/mcp")).unwrap();

    assert!(!report.value.changed);
    assert!(!report.value.oauth_dropped);
    assert!(report.is_clean());
    assert_eq!(
        std::fs::read_to_string(root.join("ivar.json")).unwrap(),
        before
    );
}

#[test]
fn editing_an_undeclared_server_is_refused_without_writing() {
    let (_guard, root) = seeded_hall();
    let before = std::fs::read_to_string(root.join("ivar.json")).unwrap();

    let failure = edit(
        &Ctx::new(root.clone()),
        &to_url("ghost", "https://x.test/mcp"),
    )
    .unwrap_err();

    assert_eq!(failure.code, "mcp.server_not_found");
    assert_eq!(
        std::fs::read_to_string(root.join("ivar.json")).unwrap(),
        before
    );
}

#[test]
fn dropping_oauth_deletes_its_orphan_secret() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    add(&ctx, &http("x", "https://old.test/mcp")).unwrap();
    with_oauth(&root, "x");
    let layout = Layout::at(root.clone());
    McpSecrets::set_and_write(&layout, "IVAR_MCP_X_SECRET", "secret-value").unwrap();

    let report = edit(&ctx, &to_url("x", "https://new.test/mcp")).unwrap();

    assert!(report.is_clean());
    assert_eq!(
        report.value.secret_removed.as_deref(),
        Some("IVAR_MCP_X_SECRET")
    );
    assert_eq!(
        McpSecrets::read(&layout).unwrap().get("IVAR_MCP_X_SECRET"),
        None
    );
}

#[test]
fn an_unchanged_edit_keeps_the_secret() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    add(&ctx, &http("x", "https://same.test/mcp")).unwrap();
    with_oauth(&root, "x");
    let layout = Layout::at(root.clone());
    McpSecrets::set_and_write(&layout, "IVAR_MCP_X_SECRET", "secret-value").unwrap();

    let report = edit(&ctx, &to_url("x", "https://same.test/mcp")).unwrap();

    assert_eq!(report.value.secret_removed, None);
    assert_eq!(
        McpSecrets::read(&layout).unwrap().get("IVAR_MCP_X_SECRET"),
        Some("secret-value")
    );
}
