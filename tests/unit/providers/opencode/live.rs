use crate::domain::mcp::CredentialState;
use crate::providers::opencode::live::parse_mcp_list;

const REAL_OPENCODE_OUTPUT: &str = r#"┌  MCP Servers
│
●  ✓ context7 connected
│      https://mcp.context7.com/mcp
│
●  ⚠ valhalla-hall-cloudflare-observability needs authentication
│      https://observability.mcp.cloudflare.com/mcp
│
●  ✓ valhalla-hall-figma connected (OAuth)
│      https://mcp.figma.com/mcp
│
└  7 server(s)
"#;

#[test]
fn parses_real_opencode_mcp_list_output() {
    let states = parse_mcp_list(REAL_OPENCODE_OUTPUT);
    assert_eq!(
        states.get("context7"),
        Some(&CredentialState::Authenticated)
    );
    assert_eq!(
        states.get("valhalla-hall-cloudflare-observability"),
        Some(&CredentialState::Missing)
    );
    assert_eq!(
        states.get("valhalla-hall-figma"),
        Some(&CredentialState::Authenticated)
    );
}

#[test]
fn unknown_opencode_status_or_failed_bullet_fails_closed_to_unknown() {
    let output = "●  ✗ broken-server failed\n●  ? weird-server something\n";
    let states = parse_mcp_list(output);
    assert_eq!(states.get("broken-server"), Some(&CredentialState::Unknown));
    assert_eq!(states.get("weird-server"), Some(&CredentialState::Unknown));
}

#[test]
fn opencode_strips_ansi_and_ignores_framing() {
    let output =
        "\x1b[32m●  ✓ server-ansi connected\x1b[0m\n│  https://example.com\n└  1 server(s)\n";
    let states = parse_mcp_list(output);
    assert_eq!(
        states.get("server-ansi"),
        Some(&CredentialState::Authenticated)
    );
}
