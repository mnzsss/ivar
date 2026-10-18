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

fn declare(ctx: &Ctx, name: &str) {
    add(
        ctx,
        &AddInput {
            name: name.to_owned(),
            url: Some(format!("https://{name}.test/mcp")),
            command: None,
            args: vec![],
        },
    )
    .unwrap();
}

fn share_secret(root: &camino::Utf8PathBuf, names: &[&str], var: &str) {
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let servers = manifest
        .mcp_servers()
        .iter()
        .cloned()
        .map(|s| {
            if names.contains(&s.name.as_str()) {
                s.oauth(McpOauth::new("c", var))
            } else {
                s
            }
        })
        .collect();
    Manifest::write(&layout, &manifest.with_mcp_servers(servers).unwrap()).unwrap();
    McpSecrets::set_and_write(&layout, var, "secret-value").unwrap();
}

#[test]
fn remove_drops_the_server_from_manifest_config_and_secrets() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    declare(&ctx, "x");
    share_secret(&root, &["x"], "IVAR_MCP_X_SECRET");

    let report = remove(
        &ctx,
        &RemoveInput {
            name: "x".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(
        report.value.secret_removed.as_deref(),
        Some("IVAR_MCP_X_SECRET")
    );
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    assert!(manifest.mcp_servers().iter().all(|s| s.name != "x"));
    let claude =
        std::fs::read_to_string(layout.mcp_config(&Provider::ClaudeCode)).unwrap_or_default();
    assert!(!claude.contains("https://x.test/mcp"), "{claude}");
    assert_eq!(
        McpSecrets::read(&layout).unwrap().get("IVAR_MCP_X_SECRET"),
        None
    );
    assert!(report.is_clean());
}

#[test]
fn a_secret_another_server_still_uses_is_kept() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    declare(&ctx, "x");
    declare(&ctx, "y");
    share_secret(&root, &["x", "y"], "IVAR_MCP_SHARED");

    let report = remove(
        &ctx,
        &RemoveInput {
            name: "x".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(report.value.secret_removed, None);
    let layout = Layout::at(root);
    assert_eq!(
        McpSecrets::read(&layout).unwrap().get("IVAR_MCP_SHARED"),
        Some("secret-value")
    );
}

#[test]
fn removing_an_undeclared_server_is_refused_without_writing() {
    let (_guard, root) = seeded_hall();
    let before = std::fs::read_to_string(root.join("ivar.json")).unwrap();

    let failure = remove(
        &Ctx::new(root.clone()),
        &RemoveInput {
            name: "ghost".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "mcp.server_not_found");
    assert_eq!(
        std::fs::read_to_string(root.join("ivar.json")).unwrap(),
        before
    );
}
