#![allow(clippy::unwrap_used, clippy::expect_used, clippy::type_complexity)]

use crate::action::mcp::auth::{AuthMethod, Preregistration, ProviderRun, flow};
use crate::domain::provider::Provider;
use crate::infra::mcp_oauth::{self, OAuthEndpoints};

#[path = "flow/pipeline.rs"]
mod pipeline;

#[test]
fn discover_oauth_endpoints_pure_parsing() {
    let header = r#"Bearer realm="Figma", resource_metadata="https://mcp.figma.com/.well-known/oauth-protected-resource""#;
    // Verify the parser extracts the resource_metadata URL from the header.
    assert_eq!(
        mcp_oauth::parse_www_authenticate_resource_metadata(header),
        Some("https://mcp.figma.com/.well-known/oauth-protected-resource".to_owned())
    );
    let resource_json = r#"{"authorization_servers":["https://www.figma.com/oauth"],"resource":"https://api.figma.com","scopes_supported":["file_read"]}"#;
    let (authorization_server, resource) =
        mcp_oauth::parse_resource_metadata(resource_json).expect("parse resource");
    assert_eq!(authorization_server, "https://www.figma.com/oauth");
    assert_eq!(resource, Some("https://api.figma.com".to_owned()));
    // Note: scopes_supported is not returned by parse_resource_metadata, it's in the auth metadata

    // And: fetching the authorization server metadata
    let auth_json = r#"{"authorization_endpoint":"https://www.figma.com/oauth/authorize","token_endpoint":"https://www.figma.com/oauth/token","scopes_supported":["file_read"]}"#;
    let endpoints = mcp_oauth::parse_authorization_metadata(auth_json).expect("parse auth");
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
fn no_token_or_secret_appears_in_serialized_output() {
    let run = ProviderRun {
        provider: Provider::Omp,
        preregistration: Preregistration::Registered {
            client_id: "client-123".to_owned(),
        },
        auth_method: AuthMethod::InternalOAuthFlow,
        command: "ivar oauth".to_owned(),
        authenticated: true,
        error: None,
    };

    let debug_str = format!("{run:?}");
    let json_str = serde_json::to_string(&run).unwrap();

    let forbidden = [
        "secret",
        "token",
        "access_token",
        "refresh_token",
        "SUPER_SECRET",
    ];
    for term in forbidden {
        assert!(
            !debug_str.to_lowercase().contains(&term.to_lowercase()),
            "debug contained {term}"
        );
        assert!(
            !json_str.to_lowercase().contains(&term.to_lowercase()),
            "json contained {term}"
        );
    }
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
#[test]
fn every_advertised_scope_reaches_the_authorize_url() {
    let endpoints = OAuthEndpoints {
        authorization_endpoint: "https://mcp.linear.app/authorize".to_owned(),
        token_endpoint: "https://mcp.linear.app/token".to_owned(),
        resource: None,
        scopes_supported: Some(vec!["read".to_owned(), "write".to_owned()]),
        registration_endpoint: None,
        client_id_metadata_document_supported: false,
    };

    let url = crate::infra::oauth::authorize_url(
        &endpoints.authorization_endpoint,
        "client-123",
        "http://127.0.0.1:19876/callback",
        &crate::infra::oauth::State("st".to_owned()),
        &crate::infra::oauth::CodeChallenge("ch".to_owned()),
        endpoints.resource.as_deref(),
        flow::scope_parameter(&endpoints).as_deref(),
    );

    assert!(
        url.contains("scope=read%20write"),
        "every advertised scope must be requested, not just the first — got {url}"
    );
}

#[test]
fn a_server_advertising_no_scopes_sends_no_scope_parameter() {
    let endpoints = OAuthEndpoints {
        authorization_endpoint: "https://mcp.figma.com/authorize".to_owned(),
        token_endpoint: "https://mcp.figma.com/token".to_owned(),
        resource: None,
        scopes_supported: None,
        registration_endpoint: None,
        client_id_metadata_document_supported: false,
    };
    assert_eq!(flow::scope_parameter(&endpoints), None);
}

#[test]
fn an_empty_scope_list_sends_no_scope_parameter() {
    let endpoints = OAuthEndpoints {
        authorization_endpoint: "https://example.com/authorize".to_owned(),
        token_endpoint: "https://example.com/token".to_owned(),
        resource: None,
        scopes_supported: Some(vec![]),
        registration_endpoint: None,
        client_id_metadata_document_supported: false,
    };
    assert_eq!(
        flow::scope_parameter(&endpoints),
        None,
        "an empty list is not a scope request for the empty string"
    );
}

#[test]
fn render_url_formats_osc8_hyperlink_on_tty() {
    let url = "https://example.com/oauth/authorize?client_id=123";
    let rendered = flow::render_url(url, true);
    assert_eq!(rendered, format!("\x1b]8;;{url}\x1b\\{url}\x1b]8;;\x1b\\"));
}

#[test]
fn render_url_returns_plain_url_off_tty() {
    let url = "https://example.com/oauth/authorize?client_id=123";
    let rendered = flow::render_url(url, false);
    assert_eq!(rendered, url);
}

#[test]
fn omp_config_is_rerendered_from_the_manifest_on_disk() {
    let (_guard, root) = crate::test_support::hall_root();
    let layout = crate::store::layout::Layout::at(&root);
    crate::infra::fs::write_text(
        &layout.manifest(),
        &serde_json::json!({
            "version": 2,
            "name": "acme",
            "integration": { "via": "local", "strategy": "squash" },
            "providers": { "available": ["omp"], "default": "omp" },
            "repos": [],
            "mcp": [ { "name": "linear", "type": "http", "url": "https://mcp.linear.app/mcp" } ],
        })
        .to_string(),
    )
    .unwrap();

    flow::rematerialise_mcp(&layout, Provider::Omp).unwrap();

    let rendered: serde_json::Value = serde_json::from_str(
        &crate::infra::fs::read_text(&layout.mcp_config(&Provider::Omp))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        rendered.pointer("/mcpServers/acme-linear/auth/credentialId"),
        Some(&serde_json::json!("mcp_oauth_ivar:acme-linear"))
    );
}
