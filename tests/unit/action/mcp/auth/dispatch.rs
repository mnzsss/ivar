#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

// -- auth_command -----------------------------------------------------------

#[test]
fn auth_command_is_claude_mcp_login_for_claude_code() {
    let command = auth_command(Provider::ClaudeCode, ["mcp", "login"], "acme-figma", None);
    assert_eq!(command.display(), "claude mcp login acme-figma");
}

#[test]
fn auth_command_is_opencode_mcp_auth_for_opencode() {
    let command = auth_command(Provider::OpenCode, ["mcp", "auth"], "acme-figma", None);
    assert_eq!(command.display(), "opencode mcp auth acme-figma");
}

#[test]
fn auth_command_carries_no_env_override_without_a_fresh_secret() {
    let command = auth_command(Provider::OpenCode, ["mcp", "auth"], "acme-figma", None);
    assert!(command.envs().is_empty());
}

/// Defect fix (`R-SECRET-HANDOFF`): a fresh registration's secret must reach
/// the dispatched child's own environment — the operator cannot have
/// exported it yet on the run that just minted it. Asserted on the built
/// `Command` only; nothing here spawns a process.
#[test]
fn auth_command_puts_a_fresh_registrations_secret_into_the_childs_environment() {
    let fresh = (
        "IVAR_MCP_ACME_FIGMA_SECRET".to_owned(),
        "top-secret".to_owned(),
    );

    let command = auth_command(
        Provider::OpenCode,
        ["mcp", "auth"],
        "acme-figma",
        Some(&fresh),
    );
    assert_eq!(
        command.envs(),
        &[(
            "IVAR_MCP_ACME_FIGMA_SECRET".to_owned(),
            "top-secret".to_owned()
        )]
    );
    // The secret must never show up in the human-readable command line.
    assert!(!command.display().contains("top-secret"));
}

// -- login_failed -----------------------------------------------------------

#[test]
fn login_failed_names_the_exit_code() {
    let failure = login_failed("claude mcp login acme-figma", Some(1));
    assert_eq!(failure.code, "mcp.auth_failed");
    assert!(failure.what.contains("exited 1"));
}

#[test]
fn login_failed_names_a_signal_death() {
    let failure = login_failed("claude mcp login acme-figma", None);
    assert!(failure.what.contains("killed by a signal"));
}
