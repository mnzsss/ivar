#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::test_support::seeded_hall;

// -- preregister_if_needed: the branches that never touch the filesystem --
//
// Every branch below returns before ever reading `layout` or `manifest` (a
// different provider, no `url`, a host off the allowlist, or a server that
// already carries `oauth`), so a `seeded_hall()` is enough scaffolding —
// none of these tests reach the network or rewrite `ivar.json`.

#[test]
fn preregistration_not_needed_for_claude_code() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let server = McpServerDef::new("figma", "sse").url("https://mcp.figma.com/mcp");

    let result = preregister_if_needed(
        &layout,
        &manifest,
        Provider::ClaudeCode,
        &server,
        "acme-figma",
    )
    .unwrap();
    assert!(matches!(result.report, Preregistration::NotNeeded));
}

#[test]
fn preregistration_not_needed_without_a_url() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let server = McpServerDef::new("linear", "stdio").command("linear-mcp");

    let result = preregister_if_needed(
        &layout,
        &manifest,
        Provider::OpenCode,
        &server,
        "acme-linear",
    )
    .unwrap();
    assert!(matches!(result.report, Preregistration::NotNeeded));
}

#[test]
fn preregistration_not_needed_for_a_host_off_the_allowlist() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let server = McpServerDef::new("linear", "sse").url("https://mcp.linear.app/mcp");

    let result = preregister_if_needed(
        &layout,
        &manifest,
        Provider::OpenCode,
        &server,
        "acme-linear",
    )
    .unwrap();
    assert!(matches!(result.report, Preregistration::NotNeeded));
}

/// R-IDEMPOTENT, the manifest half: a server whose entry already carries
/// `oauth` is skipped outright, never re-registered — no network call, no
/// rewrite of `ivar.json`.
#[test]
fn preregistration_skipped_when_the_manifest_already_carries_oauth() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    // `CARGO_MANIFEST_DIR` is a variable cargo always sets on the test
    // process itself — used here purely as "a variable guaranteed to be
    // set", to exercise the present-and-usable branch without mutating the
    // process environment (`unsafe_code` is denied in this crate, so
    // `std::env::set_var` is not an option).
    let server = McpServerDef::new("figma", "sse")
        .url("https://mcp.figma.com/mcp")
        .oauth(McpOauth::new("existing-client", "CARGO_MANIFEST_DIR"));

    let result = preregister_if_needed(
        &layout,
        &manifest,
        Provider::OpenCode,
        &server,
        "acme-figma",
    )
    .unwrap();
    assert!(matches!(result.report, Preregistration::Skipped));
    let (var, val) = result.secret.unwrap();
    assert_eq!(var, "CARGO_MANIFEST_DIR");
    assert_eq!(val, env!("CARGO_MANIFEST_DIR"));

    // Verify it backfilled into .ivar/secrets/mcp.env
    let secrets = McpSecrets::read(&layout).unwrap();
    assert_eq!(
        secrets.get("CARGO_MANIFEST_DIR"),
        Some(env!("CARGO_MANIFEST_DIR"))
    );
}

#[test]
fn preregistration_skipped_resolves_from_mcp_secrets_store_when_env_is_unset() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let var_name = "IVAR_MCP_AUTH_TEST_STORED_ONLY_VAR";
    McpSecrets::set_and_write(&layout, var_name, "stored-secret-val").unwrap();

    let server = McpServerDef::new("figma", "sse")
        .url("https://mcp.figma.com/mcp")
        .oauth(McpOauth::new("existing-client", var_name));

    let result = preregister_if_needed(
        &layout,
        &manifest,
        Provider::OpenCode,
        &server,
        "acme-figma",
    )
    .unwrap();
    assert!(matches!(result.report, Preregistration::Skipped));
    let (var, val) = result.secret.unwrap();
    assert_eq!(var, var_name);
    assert_eq!(val, "stored-secret-val");
}

/// Defect fix, related improvement (`R-ERRORS`): on the `Skipped` path
/// a missing secret in both environment and local store must fail
/// early, naming the variable — rather than dispatch into OpenCode's
/// confusing `client_secret_basic authentication requires a client_secret`.
#[test]
fn preregistration_skipped_path_fails_naming_the_variable_when_it_is_unset() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let server = McpServerDef::new("figma", "sse")
        .url("https://mcp.figma.com/mcp")
        .oauth(McpOauth::new(
            "existing-client",
            "IVAR_MCP_AUTH_TEST_DOES_NOT_EXIST_UNSET",
        ));

    let failure = preregister_if_needed(
        &layout,
        &manifest,
        Provider::OpenCode,
        &server,
        "acme-figma",
    )
    .unwrap_err();

    assert_eq!(failure.code, "mcp.missing_client_secret_env");
    assert!(
        failure
            .what
            .contains("IVAR_MCP_AUTH_TEST_DOES_NOT_EXIST_UNSET")
    );
}

// -- secret_env_var: the one place the export variable name is built -------

#[test]
fn secret_env_var_uppercases_and_folds_non_alphanumerics() {
    assert_eq!(secret_env_var("acme-figma"), "IVAR_MCP_ACME_FIGMA_SECRET");
    assert_eq!(secret_env_var("linear"), "IVAR_MCP_LINEAR_SECRET");
}

// -- host_of --------------------------------------------------------------

#[test]
fn host_of_strips_scheme_path_query_and_fragment() {
    assert_eq!(
        host_of("https://mcp.figma.com/mcp?x=1#frag"),
        Some("mcp.figma.com")
    );
}

#[test]
fn host_of_strips_port_and_userinfo() {
    assert_eq!(
        host_of("https://user:pass@mcp.figma.com:443/mcp"),
        Some("mcp.figma.com")
    );
}

#[test]
fn host_of_works_without_a_scheme() {
    assert_eq!(host_of("mcp.figma.com/mcp"), Some("mcp.figma.com"));
}
