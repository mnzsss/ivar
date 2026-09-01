// tests/unit/providers/omp/mcp.rs
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::domain::mcp::{McpOauth, McpServerDef, McpTransport};
use crate::providers::omp::mcp::server_doc;

#[test]
fn server_with_oauth_and_token_url_renders_auth_block() {
    let server = McpServerDef::new("figma", "http")
        .url("https://mcp.figma.com/mcp")
        .oauth(
            McpOauth::new("client-123", "IVAR_MCP_ACME_FIGMA_SECRET")
                .token_url("https://www.figma.com/oauth/token")
                .resource("https://api.figma.com"),
        );

    let doc = server_doc("acme-figma", &server, McpTransport::Http);
    let auth = doc.get("auth").expect("auth block must be present");

    assert_eq!(auth["type"], "oauth");
    assert_eq!(auth["clientId"], "client-123");
    assert_eq!(auth["clientSecret"], "${IVAR_MCP_ACME_FIGMA_SECRET}");
    assert_eq!(auth["tokenUrl"], "https://www.figma.com/oauth/token");
    assert_eq!(auth["resource"], "https://api.figma.com");
}

#[test]
fn server_with_oauth_and_token_url_omits_resource_when_none() {
    let server = McpServerDef::new("figma", "http")
        .url("https://mcp.figma.com/mcp")
        .oauth(
            McpOauth::new("client-123", "IVAR_MCP_ACME_FIGMA_SECRET")
                .token_url("https://www.figma.com/oauth/token"),
        );

    let doc = server_doc("acme-figma", &server, McpTransport::Http);
    let auth = doc.get("auth").expect("auth block must be present");

    assert_eq!(auth["type"], "oauth");
    assert_eq!(auth["clientId"], "client-123");
    assert_eq!(auth["clientSecret"], "${IVAR_MCP_ACME_FIGMA_SECRET}");
    assert_eq!(auth["tokenUrl"], "https://www.figma.com/oauth/token");
    assert!(auth.get("resource").is_none());
}

#[test]
fn client_secret_is_rendered_as_env_var_and_raw_secret_never_appears() {
    let raw_secret = "SUPER_SECRET_RAW_TOKEN_VALUE_XYZ";
    let env_var = "IVAR_MCP_ACME_FIGMA_SECRET";
    let server = McpServerDef::new("figma", "http")
        .url("https://mcp.figma.com/mcp")
        .oauth(McpOauth::new("client-123", env_var).token_url("https://www.figma.com/oauth/token"));

    let doc = server_doc("acme-figma", &server, McpTransport::Http);
    let serialized = serde_json::to_string(&doc).unwrap();

    assert_eq!(doc["auth"]["clientSecret"], format!("${{{env_var}}}"));
    assert!(!serialized.contains(raw_secret));
}

#[test]
fn server_without_oauth_or_without_token_url_renders_no_auth_key() {
    // 1. Server with no oauth at all
    let server_no_oauth = McpServerDef::new("linear", "http").url("https://mcp.linear.app/mcp");
    let doc1 = server_doc("acme-linear", &server_no_oauth, McpTransport::Http);
    assert!(doc1.get("auth").is_none(), "auth key must not exist");

    // 2. Server with oauth but no token_url
    let server_no_token_url = McpServerDef::new("figma", "http")
        .url("https://mcp.figma.com/mcp")
        .oauth(McpOauth::new("client-123", "IVAR_MCP_ACME_FIGMA_SECRET"));
    let doc2 = server_doc("acme-figma", &server_no_token_url, McpTransport::Http);
    assert!(
        doc2.get("auth").is_none(),
        "auth key must not exist when token_url is None"
    );
}

#[test]
fn server_url_keeps_query_string_byte_for_byte() {
    let url = "https://mcp.figma.com/mcp?tenant=123&env=prod&flag=true#anchor";
    let server = McpServerDef::new("figma", "http").url(url).oauth(
        McpOauth::new("client-123", "IVAR_MCP_ACME_FIGMA_SECRET")
            .token_url("https://www.figma.com/oauth/token"),
    );

    let doc = server_doc("acme-figma", &server, McpTransport::Http);
    assert_eq!(doc["url"], url);
}
