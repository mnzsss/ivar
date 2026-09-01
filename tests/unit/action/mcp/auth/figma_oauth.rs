#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

use crate::action::mcp::auth::{AuthMethod, Preregistration, ProviderRun};
use crate::domain::mcp::McpServerDef;
use crate::domain::name::HallName;
use crate::domain::provider::Provider;
use crate::infra::figma;
use crate::store::manifest::{Manifest, Providers};

/// Helper to build a minimal manifest with no pre-registered Figma client.
fn manifest() -> Manifest {
    Manifest::new(
        HallName::new("test").unwrap(),
        Providers::new(vec![Provider::OpenCode], Provider::OpenCode),
        vec![],
        None,
    )
    .unwrap()
}

/// Helper to build a minimal MCP server def for Figma.
fn figma_server() -> McpServerDef {
    McpServerDef {
        name: "figma-test".to_owned(),
        type_: "stdio".to_owned(),
        command: Some("figma".to_owned()),
        args: None,
        url: Some("https://mcp.figma.com/mcp".to_owned()),
        env: None,
        oauth: None,
    }
}

#[test]
fn discover_oauth_endpoints_pure_parsing() {
    // Given: a minimal WWW-Authenticate header value pointing to resource metadata
    let header = r#"Bearer realm="Figma", resource_metadata="https://mcp.figma.com/.well-known/oauth-protected-resource""#;
    let resource_url =
        figma::parse_www_authenticate_resource_metadata(header).expect("parse header");

    // When: parsing the resource metadata JSON
    let resource_json = r#"{"authorization_servers":["https://www.figma.com/oauth"],"resource":"https://api.figma.com","scopes_supported":["file_read"]}"#;
    let (authorization_server, resource) =
        figma::parse_resource_metadata(resource_json).expect("parse resource");
    assert_eq!(authorization_server, "https://www.figma.com/oauth");
    assert_eq!(resource, Some("https://api.figma.com".to_owned()));
    // Note: scopes_supported is not returned by parse_resource_metadata, it's in the auth metadata

    // And: fetching the authorization server metadata
    let auth_json = r#"{"authorization_endpoint":"https://www.figma.com/oauth/authorize","token_endpoint":"https://www.figma.com/oauth/token","scopes_supported":["file_read"]}"#;
    let endpoints = figma::parse_authorization_metadata(auth_json).expect("parse auth");
    assert_eq!(
        endpoints.authorization_endpoint,
        "https://www.figma.com/oauth/authorize"
    );
    assert_eq!(
        endpoints.token_endpoint,
        "https://www.figma.com/oauth/token"
    );
    assert_eq!(
        endpoints.scopes_supported,
        Some(vec!["file_read".to_owned()])
    );
}

#[test]
fn build_well_known_url() {
    let base = "https://www.figma.com/oauth";
    let expected = "https://www.figma.com/oauth/.well-known/oauth-authorization-server";
    assert_eq!(figma::build_well_known_url(base).unwrap(), expected);
}

#[test]
fn provider_run_command_shows_internal_flow_label() {
    // Given: a ProviderRun from internal flow
    let run = ProviderRun {
        provider: Provider::OpenCode,
        preregistration: Preregistration::NotNeeded,
        auth_method: AuthMethod::InternalOAuthFlow,
        command: "ivar oauth".to_owned(),
        authenticated: true,
        error: None,
    };

    // Then: we can format it (Display impl should exist from mod.rs)
    let output = format!("{run:?}");
    assert!(output.contains("InternalOAuthFlow"));
    assert!(output.contains("ivar oauth"));
}

#[test]
fn provider_run_command_shows_provider_command_label() {
    // Given: a ProviderRun from provider-owned flow
    let run = ProviderRun {
        provider: Provider::OpenCode,
        preregistration: Preregistration::NotNeeded,
        auth_method: AuthMethod::ProviderCommand,
        command: "opencode mcp auth figma-test".to_owned(),
        authenticated: true,
        error: None,
    };

    // Then: we can format it
    let output = format!("{run:?}");
    assert!(output.contains("ProviderCommand"));
    assert!(output.contains("opencode mcp auth figma-test"));
}
