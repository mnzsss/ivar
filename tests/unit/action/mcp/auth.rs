#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::support::declare_server;
use super::*;
use crate::action::mcp::{resolve_provider, resolve_server};
use crate::domain::mcp::McpServerDef;
use crate::error::Status;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers};
use crate::test_support::seeded_hall;

// -- resolve_server -----------------------------------------------------

#[test]
fn resolve_server_finds_a_declared_server_by_name() {
    let (_guard, root) = seeded_hall();
    declare_server(
        &root,
        McpServerDef::new("linear", "local").command("linear-mcp"),
    );
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let server = resolve_server(&manifest, "linear").unwrap();
    assert_eq!(server.name, "linear");
}

#[test]
fn resolve_server_lists_the_declared_names_when_absent() {
    let (_guard, root) = seeded_hall();
    declare_server(
        &root,
        McpServerDef::new("linear", "local").command("linear-mcp"),
    );
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let failure = resolve_server(&manifest, "figma").unwrap_err();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "mcp.server_not_found");
    assert!(failure.expected.unwrap().contains("linear"));
}

#[test]
fn resolve_server_names_the_empty_case_when_none_are_declared() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let failure = resolve_server(&manifest, "figma").unwrap_err();
    assert!(failure.expected.unwrap().contains("no servers declared"));
}

// -- resolve_provider -----------------------------------------------------

#[test]
fn resolve_provider_defaults_to_the_hall_default() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    assert_eq!(
        resolve_provider(&manifest, None).unwrap(),
        Provider::ClaudeCode
    );
}

#[test]
fn resolve_provider_honours_an_explicit_override() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    assert_eq!(
        resolve_provider(&manifest, Some("opencode")).unwrap(),
        Provider::OpenCode
    );
}

#[test]
fn resolve_provider_rejects_an_unknown_id() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let failure = resolve_provider(&manifest, Some("bogus")).unwrap_err();
    assert_eq!(failure.status, Status::Blocked);
}

// -- auth(): the failure paths that never reach dispatch ---------------------

#[test]
fn auth_refuses_an_unknown_server_before_any_dispatch() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let failure = auth(
        &ctx,
        &AuthInput {
            server: "figma".to_owned(),
            provider: None,
            all_providers: false,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "mcp.server_not_found");
}

#[test]
fn auth_refuses_an_unknown_provider_before_any_dispatch() {
    let (_guard, root) = seeded_hall();
    declare_server(
        &root,
        McpServerDef::new("linear", "local").command("linear-mcp"),
    );
    let ctx = Ctx::new(root.clone());

    let failure = auth(
        &ctx,
        &AuthInput {
            server: "linear".to_owned(),
            provider: Some("bogus".to_owned()),
            all_providers: false,
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
}

#[test]
fn auth_refuses_an_unknown_server_for_all_providers_too() {
    // `--all-providers` still resolves the server before it ever touches the
    // provider loop — an unknown name is not "0 of N providers succeeded",
    // it is the same refusal as the single-provider form.
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let failure = auth(
        &ctx,
        &AuthInput {
            server: "figma".to_owned(),
            provider: None,
            all_providers: true,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "mcp.server_not_found");
}

#[test]
fn all_providers_hall_lists_available_providers_in_declared_order() {
    // Grounds the ordering claim in the actual manifest a two-provider hall
    // produces — `auth`'s `.map` over this exact sequence is what makes
    // `--all-providers` "sequential, in order" rather than an assumption.
    let (_guard, root) = seeded_hall();
    let ctx = crate::action::Ctx::new(root.clone());
    crate::action::provider::add::add(
        &ctx,
        &crate::action::provider::add::AddInput {
            name: "opencode".to_owned(),
        },
    )
    .unwrap();

    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    assert_eq!(
        manifest.providers().available(),
        &[Provider::ClaudeCode, Provider::OpenCode]
    );
}

#[test]
fn provider_order_puts_claude_code_first_if_present() {
    let available = vec![Provider::OpenCode, Provider::ClaudeCode, Provider::Omp];
    let order = provider_order(&available);
    assert_eq!(
        order,
        vec![Provider::ClaudeCode, Provider::OpenCode, Provider::Omp]
    );
}

#[test]
fn provider_order_for_a_hall_listing_t3_never_probes_it_twice() {
    let owners: Vec<Provider> = Providers::new(
        vec![Provider::ClaudeCode, Provider::T3],
        Provider::ClaudeCode,
    )
    .config_owners()
    .collect();
    assert_eq!(provider_order(&owners), vec![Provider::ClaudeCode]);
}

#[test]
fn provider_order_preserves_order_if_claude_code_absent() {
    let available = vec![Provider::OpenCode, Provider::Omp];
    let order = provider_order(&available);
    assert_eq!(order, vec![Provider::OpenCode, Provider::Omp]);
}
