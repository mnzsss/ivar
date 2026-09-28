use crate::domain::mcp::CredentialState;
use crate::providers::claude_code::live::parse_mcp_list;
use std::collections::BTreeMap;

const REAL_CLAUDE_OUTPUT: &str = r#"Checking MCP server health…

claude.ai Claude Docs: https://api.anthropic.com/v1/pages/mcp - ✔ Connected
ai-memory: ai-memory mcp-bridge --server-url http://127.0.0.1:49374/mcp - ✔ Connected
valhalla-hall-cloudflare-docs: https://docs.mcp.cloudflare.com/mcp (HTTP) - ✔ Connected
valhalla-hall-figma: https://mcp.figma.com/mcp (HTTP) - ! Needs authentication
valhalla-hall-graph: ivar graph mcp - ✔ Connected
valhalla-hall-linear: https://mcp.linear.app/mcp (HTTP) - ✔ Connected
"#;

#[test]
fn parses_real_claude_mcp_list_output() {
    let states = parse_mcp_list(REAL_CLAUDE_OUTPUT);
    assert_eq!(
        states.get("claude.ai Claude Docs"),
        Some(&CredentialState::Authenticated)
    );
    assert_eq!(
        states.get("ai-memory"),
        Some(&CredentialState::Authenticated)
    );
    assert_eq!(
        states.get("valhalla-hall-cloudflare-docs"),
        Some(&CredentialState::Authenticated)
    );
    assert_eq!(
        states.get("valhalla-hall-figma"),
        Some(&CredentialState::Missing)
    );
    assert_eq!(
        states.get("valhalla-hall-graph"),
        Some(&CredentialState::Authenticated)
    );
    assert_eq!(
        states.get("valhalla-hall-linear"),
        Some(&CredentialState::Authenticated)
    );
}

#[test]
fn unknown_claude_status_fails_closed_to_unknown() {
    let output =
        "server-one: https://example.com - ✗ Failed to connect\nserver-two: local - ? Pending\n";
    let states = parse_mcp_list(output);
    assert_eq!(states.get("server-one"), Some(&CredentialState::Unknown));
    assert_eq!(states.get("server-two"), Some(&CredentialState::Unknown));
}

#[test]
fn claude_empty_or_banner_only_yields_empty_map() {
    assert_eq!(parse_mcp_list(""), BTreeMap::new());
    assert_eq!(
        parse_mcp_list("Checking MCP server health…\n"),
        BTreeMap::new()
    );
}
