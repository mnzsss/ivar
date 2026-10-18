#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::Ctx;
use crate::action::hall::{self, InitInput};
use crate::domain::mcp::McpOauth;
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::test_support::{hall_root, seeded_hall};

/// `seeded_hall` initialises with the default provider (Claude Code); the
/// Figma-on-OpenCode path needs an OpenCode hall.
fn opencode_hall() -> (tempfile::TempDir, camino::Utf8PathBuf) {
    let (guard, root) = hall_root();
    hall::init(
        &Ctx::new(root.clone()),
        &InitInput {
            path: camino::Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: Some("opencode".to_owned()),
        },
    )
    .unwrap();
    (guard, root)
}

#[test]
fn add_figma_preset_writes_manifest_and_materialises_opencode_config() {
    let (_guard, root) = opencode_hall();
    let ctx = Ctx::new(root.clone());
    let input = AddInput {
        name: "figma".to_owned(),
        url: None,
        command: None,
        args: vec![],
    };

    let report = add(&ctx, &input).expect("add figma should succeed");
    let outcome = report.value;

    assert_eq!(outcome.name, "figma");
    assert_eq!(outcome.url.as_deref(), Some("https://mcp.figma.com/mcp"));
    assert!(outcome.changed);
    assert_eq!(outcome.next, "ivar mcp auth figma --provider opencode");

    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let server = manifest
        .mcp_servers()
        .iter()
        .find(|s| s.name == "figma")
        .expect("figma server in manifest");
    assert_eq!(server.type_, "http");
    assert_eq!(server.url.as_deref(), Some("https://mcp.figma.com/mcp"));

    let opencode_path = layout.mcp_config(&Provider::OpenCode);
    let opencode_raw = std::fs::read_to_string(opencode_path).unwrap();
    assert!(
        opencode_raw.contains("acme-figma"),
        "opencode.json must contain materialised server name `acme-figma`"
    );
}

#[test]
fn add_custom_url_writes_entry_and_materialises() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    let input = AddInput {
        name: "custom".to_owned(),
        url: Some("https://mcp.example.com/sse".to_owned()),
        command: None,
        args: vec![],
    };

    let report = add(&ctx, &input).expect("add custom server should succeed");
    let outcome = report.value;
    assert_eq!(outcome.name, "custom");
    assert_eq!(outcome.url.as_deref(), Some("https://mcp.example.com/sse"));
    assert!(outcome.changed);

    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let server = manifest
        .mcp_servers()
        .iter()
        .find(|s| s.name == "custom")
        .expect("custom server in manifest");
    assert_eq!(server.url.as_deref(), Some("https://mcp.example.com/sse"));
}

#[test]
fn add_unknown_preset_without_url_is_blocked_and_leaves_manifest_untouched() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    let input = AddInput {
        name: "slack".to_owned(),
        url: None,
        command: None,
        args: vec![],
    };

    let err = add(&ctx, &input).expect_err("unknown preset must be blocked");
    assert_eq!(err.status, Status::Blocked);
    assert_eq!(err.code, "mcp.unknown_preset");

    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    assert!(manifest.mcp_servers().is_empty());
}

#[test]
fn add_same_definition_is_idempotent_and_reports_changed_false() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    let input = AddInput {
        name: "figma".to_owned(),
        url: None,
        command: None,
        args: vec![],
    };

    let first = add(&ctx, &input).unwrap().value;
    assert!(first.changed);
    let second = add(&ctx, &input).unwrap().value;
    assert_eq!(second.name, "figma");
    assert_eq!(second.url.as_deref(), Some("https://mcp.figma.com/mcp"));
    assert!(!second.changed);
}

/// `ivar mcp auth` later writes an `oauth` block onto the entry. That block
/// is generated metadata, not part of what `add` declares, so re-running
/// `add` after authenticating is still a no-op rather than a conflict.
#[test]
fn add_after_auth_metadata_is_still_idempotent() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    let input = AddInput {
        name: "figma".to_owned(),
        url: None,
        command: None,
        args: vec![],
    };
    add(&ctx, &input).unwrap();

    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let servers = manifest
        .mcp_servers()
        .iter()
        .map(|s| s.clone().oauth(McpOauth::public("cid")))
        .collect();
    Manifest::write(&layout, &manifest.with_mcp_servers(servers).unwrap()).unwrap();
    let before = std::fs::read_to_string(root.join("ivar.json")).unwrap();

    assert!(!add(&ctx, &input).unwrap().value.changed);
    assert_eq!(
        std::fs::read_to_string(root.join("ivar.json")).unwrap(),
        before
    );
}

#[test]
fn add_conflicting_definition_is_blocked_and_leaves_manifest_untouched() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    let first_input = AddInput {
        name: "myserver".to_owned(),
        url: Some("https://mcp.one.com/mcp".to_owned()),
        command: None,
        args: vec![],
    };
    add(&ctx, &first_input).unwrap();

    let conflicting_input = AddInput {
        name: "myserver".to_owned(),
        url: Some("https://mcp.two.com/mcp".to_owned()),
        command: None,
        args: vec![],
    };
    let err = add(&ctx, &conflicting_input).expect_err("conflicting server must be blocked");
    assert_eq!(err.status, Status::Blocked);
    assert_eq!(err.code, "mcp.server_conflict");

    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let server = manifest
        .mcp_servers()
        .iter()
        .find(|s| s.name == "myserver")
        .unwrap();
    assert_eq!(server.url.as_deref(), Some("https://mcp.one.com/mcp"));
}

fn stdio(name: &str, command: &str, args: &[&str]) -> AddInput {
    AddInput {
        name: name.to_owned(),
        url: None,
        command: Some(command.to_owned()),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
    }
}

#[test]
fn add_stdio_declares_a_local_server_and_materialises_it() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let outcome = add(&ctx, &stdio("files", "npx", &["-y", "@mcp/fs"]))
        .unwrap()
        .value;

    assert!(outcome.changed);
    assert_eq!(outcome.url, None);
    assert_eq!(outcome.command.as_deref(), Some("npx"));
    assert_eq!(outcome.args, vec!["-y".to_owned(), "@mcp/fs".to_owned()]);
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let server = manifest
        .mcp_servers()
        .iter()
        .find(|s| s.name == "files")
        .unwrap();
    assert_eq!(server.type_, "local");
    assert_eq!(server.command.as_deref(), Some("npx"));
    assert_eq!(
        server.args.as_deref(),
        Some(&["-y".to_owned(), "@mcp/fs".to_owned()][..])
    );
    let claude = std::fs::read_to_string(layout.mcp_config(&Provider::ClaudeCode)).unwrap();
    assert!(claude.contains("\"npx\""), "{claude}");
    assert!(claude.contains("@mcp/fs"), "{claude}");
}

#[test]
fn adding_the_same_stdio_server_twice_changes_nothing() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    add(&ctx, &stdio("files", "npx", &["@mcp/fs"])).unwrap();
    let before = std::fs::read_to_string(root.join("ivar.json")).unwrap();

    let outcome = add(&ctx, &stdio("files", "npx", &["@mcp/fs"]))
        .unwrap()
        .value;

    assert!(!outcome.changed);
    assert_eq!(
        std::fs::read_to_string(root.join("ivar.json")).unwrap(),
        before
    );
}

#[test]
fn the_same_name_with_other_args_is_a_conflict() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    add(&ctx, &stdio("files", "npx", &["@mcp/fs"])).unwrap();

    let failure = add(&ctx, &stdio("files", "npx", &["@mcp/other"])).unwrap_err();

    assert_eq!(failure.code, "mcp.server_conflict");
}
