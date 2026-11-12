#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::mcp::auth::support::declare_server;
use crate::infra::mcp_oauth::OAuthEndpoints;
use crate::test_support::seeded_hall;

/// `R-AUTH-LIFECYCLE`: the endpoints a fresh registration persists are the ones
/// discovery returned. Hardcoding any of them — the failure this guards — would
/// send omp's refresh at the wrong token endpoint and silently break renewal.
#[test]
fn fresh_registration_persists_the_endpoints_discovery_returned() {
    let discovered = crate::infra::mcp_oauth::OAuthEndpoints {
        authorization_endpoint: "https://auth.example.com/authorize".to_owned(),
        token_endpoint: "https://auth.example.com/oauth/token?tenant=42".to_owned(),
        resource: Some("https://api.example.com/mcp".to_owned()),
        scopes_supported: None,
        registration_endpoint: None,
        client_id_metadata_document_supported: false,
    };

    let oauth = oauth_registration(
        "client-abc",
        Some("IVAR_MCP_ACME_SECRET"),
        Some(&discovered),
    );

    assert_eq!(oauth.client_id, "client-abc");
    assert_eq!(
        oauth.client_secret_env.as_deref(),
        Some("IVAR_MCP_ACME_SECRET")
    );
    // Byte-for-byte, query string included: a token endpoint is not a host.
    assert_eq!(
        oauth.token_url.as_deref(),
        Some("https://auth.example.com/oauth/token?tenant=42")
    );
    assert_eq!(
        oauth.resource.as_deref(),
        Some("https://api.example.com/mcp")
    );
}

/// A server whose metadata publishes no `resource` (RFC 8707 is not universal)
/// must persist none — a fabricated identifier would be rejected at the token
/// endpoint.
#[test]
fn fresh_registration_persists_no_resource_when_discovery_returned_none() {
    let discovered = crate::infra::mcp_oauth::OAuthEndpoints {
        authorization_endpoint: "https://auth.example.com/authorize".to_owned(),
        token_endpoint: "https://auth.example.com/oauth/token".to_owned(),
        resource: None,
        scopes_supported: None,
        registration_endpoint: None,
        client_id_metadata_document_supported: false,
    };

    let oauth = oauth_registration(
        "client-abc",
        Some("IVAR_MCP_ACME_SECRET"),
        Some(&discovered),
    );

    assert_eq!(
        oauth.token_url.as_deref(),
        Some("https://auth.example.com/oauth/token")
    );
    assert_eq!(oauth.resource, None);
}

#[test]
fn preregistration_not_needed_without_a_url() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let server = McpServerDef::new("linear", "local").command("linear-mcp");

    let result = preregister_if_needed(
        &layout,
        &manifest,
        Provider::OpenCode,
        &server,
        "acme-linear",
        None,
    )
    .unwrap();
    assert!(matches!(result.report, Preregistration::NotNeeded));
}
#[test]
fn preregistration_not_needed_for_a_host_off_the_allowlist() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let server = McpServerDef::new("linear", "http").url("https://mcp.linear.app/mcp");

    let result = preregister_if_needed(
        &layout,
        &manifest,
        Provider::OpenCode,
        &server,
        "acme-linear",
        None,
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
    let var_name = "IVAR_MCP_AUTH_TEST_ALREADY_REGISTERED_VAR";
    McpSecrets::set_and_write(&layout, var_name, "stored-secret-val").unwrap();
    let server = McpServerDef::new("figma", "http")
        .url("https://mcp.figma.com/mcp")
        .oauth(McpOauth::new("existing-client", var_name));

    let result = preregister_if_needed(
        &layout,
        &manifest,
        Provider::OpenCode,
        &server,
        "acme-figma",
        None,
    )
    .unwrap();

    assert!(matches!(result.report, Preregistration::Skipped));
    assert_eq!(result.client_id.as_deref(), Some("existing-client"));
    assert_eq!(
        result.secret,
        Some((var_name.to_owned(), "stored-secret-val".to_owned()))
    );
}

#[test]
fn secret_from_the_environment_wins_and_is_backfilled_into_the_mcp_secrets_store() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let var_name = "IVAR_MCP_AUTH_TEST_ENV_BACKFILL_VAR";
    McpSecrets::set_and_write(&layout, var_name, "stale-stored-val").unwrap();

    let val = resolve_secret(&layout, var_name, Some("from-env".to_owned()), "figma").unwrap();

    assert_eq!(val, "from-env");
    let secrets = McpSecrets::read(&layout).unwrap();
    assert_eq!(secrets.get(var_name), Some("from-env"));
}

#[test]
fn preregistration_skipped_resolves_from_mcp_secrets_store_when_env_is_unset() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let var_name = "IVAR_MCP_AUTH_TEST_STORED_ONLY_VAR";
    McpSecrets::set_and_write(&layout, var_name, "stored-secret-val").unwrap();

    let server = McpServerDef::new("figma", "http")
        .url("https://mcp.figma.com/mcp")
        .oauth(McpOauth::new("existing-client", var_name));

    let result = preregister_if_needed(
        &layout,
        &manifest,
        Provider::OpenCode,
        &server,
        "acme-figma",
        None,
    )
    .unwrap();
    let (var, val) = result.secret.unwrap();
    assert_eq!(var, var_name);
    assert_eq!(val, "stored-secret-val");
}

#[test]
fn preregistration_skipped_leaves_existing_endpoints_intact_and_does_not_overwrite() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let var_name = "IVAR_MCP_AUTH_TEST_EXISTING_ENDPOINTS_VAR";
    McpSecrets::set_and_write(&layout, var_name, "stored-secret-val").unwrap();

    let original_oauth = McpOauth::new("existing-client", var_name)
        .token_url("https://custom.auth.server/oauth/token")
        .resource("https://custom.api.server");

    let server = McpServerDef::new("figma", "http")
        .url("https://mcp.figma.com/mcp")
        .oauth(original_oauth.clone());

    let manifest = Manifest::read(&layout)
        .unwrap()
        .unwrap()
        .with_mcp_servers(vec![server.clone()])
        .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    let result = preregister_if_needed(
        &layout,
        &manifest,
        Provider::OpenCode,
        &server,
        "acme-figma",
        None,
    )
    .unwrap();
    assert!(matches!(result.report, Preregistration::Skipped));

    // Verify the stored manifest was not modified or overwritten
    let manifest_after = Manifest::read(&layout).unwrap().unwrap();
    let server_after = manifest_after
        .mcp_servers()
        .first()
        .expect("server must exist");
    assert_eq!(server_after.oauth.as_ref(), Some(&original_oauth));
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
    let server = McpServerDef::new("figma", "http")
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
        None,
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

#[test]
fn a_public_registration_records_no_secret_variable() {
    let discovered = crate::infra::mcp_oauth::OAuthEndpoints {
        authorization_endpoint: "https://mcp.linear.app/authorize".to_owned(),
        token_endpoint: "https://mcp.linear.app/token".to_owned(),
        resource: None,
        scopes_supported: Some(vec!["read".to_owned(), "write".to_owned()]),
        registration_endpoint: Some("https://mcp.linear.app/register".to_owned()),
        client_id_metadata_document_supported: false,
    };

    let oauth = oauth_registration("Uk1eiX5O6ndVHwo_", None, Some(&discovered));

    assert_eq!(oauth.client_id, "Uk1eiX5O6ndVHwo_");
    assert_eq!(
        oauth.client_secret_env, None,
        "a public client has no secret, so it names no variable"
    );
    assert_eq!(
        oauth.token_url.as_deref(),
        Some("https://mcp.linear.app/token"),
        "token_url is what makes omp's native refresh work"
    );
}

#[test]
fn a_confidential_registration_still_records_its_variable() {
    let oauth = oauth_registration("client-123", Some("IVAR_MCP_ACME_SECRET"), None);
    assert_eq!(
        oauth.client_secret_env.as_deref(),
        Some("IVAR_MCP_ACME_SECRET")
    );
    assert_eq!(oauth.token_url, None);
}

#[test]
fn a_recorded_public_client_re_runs_without_resolving_a_secret() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let server = McpServerDef::new("linear", "http")
        .url("https://mcp.linear.app/mcp")
        .oauth(McpOauth::public("Uk1eiX5O6ndVHwo_"));

    let result = preregister_if_needed(
        &layout,
        &manifest,
        Provider::Omp,
        &server,
        "acme-linear",
        None,
    )
    .expect("a recorded public client must re-run without a secret");

    assert_eq!(result.auth_mode, AuthMode::None);
    assert_eq!(result.secret, None);
    assert_eq!(result.client_id.as_deref(), Some("Uk1eiX5O6ndVHwo_"));
    assert!(matches!(result.report, Preregistration::Skipped));
}

fn prisda_endpoints(registration_endpoint: Option<&str>) -> OAuthEndpoints {
    OAuthEndpoints {
        authorization_endpoint: "https://mcp.prisda.test/api/auth/oauth2/authorize".to_owned(),
        token_endpoint: "https://mcp.prisda.test/api/auth/oauth2/token".to_owned(),
        resource: Some("https://mcp.prisda.test/mcp".to_owned()),
        scopes_supported: Some(vec!["cms:write".to_owned()]),
        registration_endpoint: registration_endpoint.map(str::to_owned),
        client_id_metadata_document_supported: true,
    }
}

#[test]
fn a_server_that_accepts_metadata_documents_gets_the_ivar_document_as_a_public_client() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let server = McpServerDef::new("prisda", "http").url("https://mcp.prisda.test/mcp");
    declare_server(&root, server.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let result = preregister_if_needed(
        &layout,
        &manifest,
        Provider::Omp,
        &server,
        "acme-prisda",
        Some(&prisda_endpoints(None)),
    )
    .expect("a CIMD server needs no registration");

    assert_eq!(result.client_id.as_deref(), Some(CIMD_CLIENT_ID));
    assert_eq!(result.auth_mode, AuthMode::None);
    assert_eq!(result.secret, None);
    assert!(matches!(
        &result.report,
        Preregistration::MetadataDocument { client_id } if client_id == CIMD_CLIENT_ID
    ));
    assert!(!root.join(".ivar/secrets/mcp.env").exists());

    let persisted = Manifest::read(&layout).unwrap().unwrap().mcp_servers()[0]
        .oauth
        .clone()
        .expect("the CIMD client is recorded so omp can refresh");
    assert_eq!(persisted.client_id, CIMD_CLIENT_ID);
    assert_eq!(persisted.client_secret_env, None);
    assert_eq!(
        persisted.token_url.as_deref(),
        Some("https://mcp.prisda.test/api/auth/oauth2/token")
    );
    assert_eq!(
        persisted.resource.as_deref(),
        Some("https://mcp.prisda.test/mcp")
    );

    let reread = Manifest::read(&layout).unwrap().unwrap();
    let second = preregister_if_needed(
        &layout,
        &reread,
        Provider::Omp,
        &reread.mcp_servers()[0],
        "acme-prisda",
        Some(&prisda_endpoints(None)),
    )
    .unwrap();
    assert!(matches!(second.report, Preregistration::Skipped));
    assert_eq!(second.client_id.as_deref(), Some(CIMD_CLIENT_ID));

    let omp_config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(layout.mcp_config(&Provider::Omp)).unwrap())
            .unwrap();
    let servers = omp_config["mcpServers"].as_object().unwrap();
    assert_eq!(servers.len(), 1);
    let auth = servers.values().next().unwrap()["auth"]
        .as_object()
        .unwrap();
    assert_eq!(auth["clientId"], CIMD_CLIENT_ID);
    assert!(!auth.contains_key("clientSecret"));
}

#[test]
fn a_metadata_document_is_preferred_over_dynamic_registration() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let server = McpServerDef::new("prisda", "http").url("https://mcp.prisda.test/mcp");
    declare_server(&root, server.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let unreachable_registration = "http://127.0.0.1:9/register";

    let result = preregister_if_needed(
        &layout,
        &manifest,
        Provider::Omp,
        &server,
        "acme-prisda",
        Some(&prisda_endpoints(Some(unreachable_registration))),
    )
    .expect("CIMD must win before any registration request is sent");

    assert!(matches!(
        result.report,
        Preregistration::MetadataDocument { .. }
    ));
}

#[test]
fn dynamic_registration_still_runs_when_metadata_documents_are_not_advertised() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let server = McpServerDef::new("linear", "http").url("https://mcp.linear.app/mcp");
    declare_server(&root, server.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let mut endpoints = prisda_endpoints(Some("http://127.0.0.1:9/register"));
    endpoints.client_id_metadata_document_supported = false;

    let failure = preregister_if_needed(
        &layout,
        &manifest,
        Provider::Omp,
        &server,
        "acme-linear",
        Some(&endpoints),
    )
    .expect_err("without CIMD the registration endpoint is tried, and nothing listens on port 9");

    assert_eq!(failure.code, "mcp_oauth.register_client");
    assert!(
        Manifest::read(&layout).unwrap().unwrap().mcp_servers()[0]
            .oauth
            .is_none()
    );
}

#[test]
fn the_provider_owned_path_never_takes_the_metadata_document_branch() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let server = McpServerDef::new("prisda", "http").url("https://mcp.prisda.test/mcp");
    declare_server(&root, server.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let result = preregister_if_needed(
        &layout,
        &manifest,
        Provider::OpenCode,
        &server,
        "acme-prisda",
        None,
    )
    .unwrap();

    assert!(matches!(result.report, Preregistration::NotNeeded));
    assert!(
        Manifest::read(&layout).unwrap().unwrap().mcp_servers()[0]
            .oauth
            .is_none()
    );
}
