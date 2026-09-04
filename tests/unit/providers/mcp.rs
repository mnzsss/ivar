// tests/unit/providers/mcp.rs
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::domain::mcp::{McpOauth, McpServerDef};
use crate::domain::provider::Provider;
use crate::providers::{mcp_root_key, mcp_server_doc};
use std::collections::BTreeMap;

#[test]
fn claude_code_characterization_stdio_server() {
    let mut env = BTreeMap::new();
    env.insert("TOKEN".to_owned(), "{env:TOKEN}".to_owned());
    let server = McpServerDef::new("docs", "stdio")
        .command("npx")
        .args(vec!["-y".to_owned(), "@acme/docs-mcp".to_owned()])
        .env(env);

    let rendered = mcp_server_doc(Provider::ClaudeCode, "acme-docs", &server);

    assert_eq!(
        rendered,
        serde_json::json!({
            "type": "stdio",
            "command": "npx",
            "args": ["-y", "@acme/docs-mcp"],
            "env": { "TOKEN": "{env:TOKEN}" }
        })
    );
    assert_eq!(mcp_root_key(Provider::ClaudeCode), "mcpServers");
}

#[test]
fn opencode_characterization_stdio_and_oauth_server() {
    let mut env = BTreeMap::new();
    env.insert("TOKEN".to_owned(), "{env:TOKEN}".to_owned());
    let stdio_server = McpServerDef::new("docs", "stdio")
        .command("npx")
        .args(vec!["-y".to_owned(), "@acme/docs-mcp".to_owned()])
        .env(env);

    let rendered_stdio = mcp_server_doc(Provider::OpenCode, "acme-docs", &stdio_server);

    assert_eq!(
        rendered_stdio,
        serde_json::json!({
            "type": "local",
            "command": ["npx", "-y", "@acme/docs-mcp"],
            "environment": { "TOKEN": "{env:TOKEN}" }
        })
    );

    let oauth_server = McpServerDef::new("figma", "sse")
        .url("https://mcp.figma.com/mcp")
        .oauth(McpOauth::new("client-123", "IVAR_MCP_ACME_FIGMA_SECRET"));

    let rendered_oauth = mcp_server_doc(Provider::OpenCode, "acme-figma", &oauth_server);

    assert_eq!(
        rendered_oauth,
        serde_json::json!({
            "type": "remote",
            "url": "https://mcp.figma.com/mcp",
            "oauth": {
                "clientId": "client-123",
                "clientSecret": "{env:IVAR_MCP_ACME_FIGMA_SECRET}",
                "redirectUri": "http://127.0.0.1:19876/callback"
            }
        })
    );
    assert_eq!(mcp_root_key(Provider::OpenCode), "mcp");
}

#[test]
fn omp_renders_stdio_server_with_command_args_and_env() {
    let mut env = BTreeMap::new();
    env.insert("API_KEY".to_owned(), "{env:API_KEY}".to_owned());
    let server = McpServerDef::new("local-tool", "stdio")
        .command("python")
        .args(vec!["-m".to_owned(), "my_mcp_server".to_owned()])
        .env(env);

    let rendered = mcp_server_doc(Provider::Omp, "acme-local-tool", &server);

    assert_eq!(
        rendered,
        serde_json::json!({
            "type": "stdio",
            "command": "python",
            "args": ["-m", "my_mcp_server"],
            "env": { "API_KEY": "{env:API_KEY}" }
        })
    );
    assert_eq!(mcp_root_key(Provider::Omp), "mcpServers");
}

#[test]
fn omp_renders_http_server_preserving_query_string_verbatim() {
    let exact_url = "https://mcp.example.com/api/v1/sse?org_id=org_123&tenant=prod#anchor";
    let server = McpServerDef::new("remote-http", "streamable-http").url(exact_url);

    let rendered = mcp_server_doc(Provider::Omp, "acme-remote-http", &server);

    assert_eq!(
        rendered,
        serde_json::json!({
            "type": "http",
            "url": exact_url
        })
    );
}

#[test]
fn omp_renders_sse_server_preserving_url_verbatim() {
    let exact_url = "https://mcp.figma.com/mcp?session=abc123xyz";
    let server = McpServerDef::new("remote-sse", "sse").url(exact_url);

    let rendered = mcp_server_doc(Provider::Omp, "acme-remote-sse", &server);

    assert_eq!(
        rendered,
        serde_json::json!({
            "type": "sse",
            "url": exact_url
        })
    );
}
