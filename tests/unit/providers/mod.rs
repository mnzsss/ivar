// tests/unit/providers/mod.rs
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::domain::provider::Provider;
use crate::providers::{self, Capabilities};

#[test]
fn launch_contract_returns_correct_binary_and_capabilities_for_all_providers() {
    let claude = providers::launch_contract(Provider::ClaudeCode);
    assert_eq!(claude.binary, "claude");
    assert_eq!(
        claude.capabilities,
        Capabilities {
            supports_resume: true,
            supports_review: true,
            interactive: true,
        }
    );

    let opencode = providers::launch_contract(Provider::OpenCode);
    assert_eq!(opencode.binary, "opencode");
    assert_eq!(
        opencode.capabilities,
        Capabilities {
            supports_resume: true,
            supports_review: false,
            interactive: true,
        }
    );

    let omp = providers::launch_contract(Provider::Omp);
    assert_eq!(omp.binary, "omp");
    assert_eq!(
        omp.capabilities,
        Capabilities {
            supports_resume: true,
            supports_review: false,
            interactive: true,
        }
    );
}

#[test]
fn claude_code_fresh_start_and_resume_commands() {
    let fresh = providers::start_command(Provider::ClaudeCode, false, &[]).unwrap();
    assert!(fresh.display().starts_with("claude"));

    let resumed = providers::start_command(Provider::ClaudeCode, true, &[]).unwrap();
    let display = resumed.display();
    assert!(display.starts_with("claude"), "was: {display}");
    assert!(display.contains("--continue"), "was: {display}");
}

#[test]
fn opencode_fresh_start_and_resume_commands() {
    let fresh = providers::start_command(Provider::OpenCode, false, &[]).unwrap();
    assert_eq!(fresh.display(), "opencode");

    let resumed = providers::start_command(Provider::OpenCode, true, &[]).unwrap();
    let display = resumed.display();
    assert!(display.starts_with("opencode"), "was: {display}");
    assert!(display.contains("--continue"), "was: {display}");
}

#[test]
fn omp_fresh_start_and_resume_commands() {
    let fresh = providers::start_command(Provider::Omp, false, &[]).unwrap();
    assert_eq!(fresh.display(), "omp");

    let resumed = providers::start_command(Provider::Omp, true, &[]).unwrap();
    let display = resumed.display();
    assert!(display.starts_with("omp"), "was: {display}");
    assert!(display.contains("--continue"), "was: {display}");
}

// -- MCP allowlist (`--settings`) -----------------------------------------

/// The `enabledMcpjsonServers` value carried by the command's `--settings`
/// argument — the set of MCP servers Claude will treat as pre-approved.
fn approved_servers(command: &crate::infra::proc::Command) -> serde_json::Value {
    let args = command.arguments();
    let flag = args
        .iter()
        .position(|a| a == "--settings")
        .expect("claude argv must carry --settings");
    let raw = args.get(flag + 1).expect("--settings must carry a value");
    let settings: serde_json::Value =
        serde_json::from_str(raw).expect("--settings value must be valid JSON");
    settings
        .get("enabledMcpjsonServers")
        .cloned()
        .expect("--settings must carry enabledMcpjsonServers")
}

/// `R-CLAUDE-ALLOWLIST`: a fresh Claude start approves exactly the
/// hall-qualified servers it is given, and nothing else.
#[test]
fn claude_fresh_start_carries_the_allowlist_in_settings() {
    let allowlist = vec!["acme-figma".to_owned(), "acme-github".to_owned()];
    let command = providers::start_command(Provider::ClaudeCode, false, &allowlist).unwrap();

    assert_eq!(
        approved_servers(&command),
        serde_json::json!(["acme-figma", "acme-github"])
    );
}

/// The allowlist survives a resume: `--continue` and `--settings` are not
/// alternatives, so a resumed session approves the same servers.
#[test]
fn claude_resume_carries_continue_and_the_allowlist() {
    let allowlist = vec!["acme-figma".to_owned()];
    let command = providers::start_command(Provider::ClaudeCode, true, &allowlist).unwrap();

    assert!(command.arguments().iter().any(|a| a == "--continue"));
    assert_eq!(
        approved_servers(&command),
        serde_json::json!(["acme-figma"])
    );
}

/// `R-CLAUDE-EMPTY`: a hall declaring no MCP servers still passes the flag,
/// with an empty list. Omitting it would let Claude fall back to prompting
/// for project servers Ivar never declared.
#[test]
fn claude_empty_allowlist_still_passes_an_explicit_empty_list() {
    let command = providers::start_command(Provider::ClaudeCode, false, &[]).unwrap();

    assert_eq!(approved_servers(&command), serde_json::json!([]));
}

/// The allowlist is passed through as given: ordering is the caller's
/// responsibility, so the harness never silently reorders an approval set.
#[test]
fn claude_allowlist_is_passed_through_in_the_order_given() {
    let allowlist = vec![
        "acme-zeta".to_owned(),
        "acme-alpha".to_owned(),
        "acme-figma".to_owned(),
    ];
    let command = providers::start_command(Provider::ClaudeCode, false, &allowlist).unwrap();

    assert_eq!(
        approved_servers(&command),
        serde_json::json!(["acme-zeta", "acme-alpha", "acme-figma"])
    );
}

/// OpenCode and OMP argv are byte-identical regardless of the allowlist:
/// `--settings` is Claude's flag alone.
#[test]
fn other_providers_never_receive_settings() {
    let allowlist = vec!["acme-figma".to_owned()];
    for provider in [Provider::OpenCode, Provider::Omp] {
        for resume in [false, true] {
            let with = providers::start_command(provider, resume, &allowlist).unwrap();
            let without = providers::start_command(provider, resume, &[]).unwrap();
            assert_eq!(with.display(), without.display());
            assert!(!with.arguments().iter().any(|a| a == "--settings"));
        }
    }
}

#[test]
fn session_projections_for_all_providers() {
    use camino::Utf8PathBuf;
    use crate::providers::SessionProjection;

    // Claude Code projects only its commands catalog
    assert_eq!(
        providers::session_projections(Provider::ClaudeCode),
        vec![SessionProjection {
            hall_source: Utf8PathBuf::from(".claude/commands"),
            config_relative_dest: Utf8PathBuf::from("commands"),
        }]
    );

    // OpenCode projects only its commands catalog
    assert_eq!(
        providers::session_projections(Provider::OpenCode),
        vec![SessionProjection {
            hall_source: Utf8PathBuf::from(".opencode/commands"),
            config_relative_dest: Utf8PathBuf::from("commands"),
        }]
    );

    // OMP projects commands catalog, hooks/pre, and extensions
    assert_eq!(
        providers::session_projections(Provider::Omp),
        vec![
            SessionProjection {
                hall_source: Utf8PathBuf::from(".omp/commands"),
                config_relative_dest: Utf8PathBuf::from("commands"),
            },
            SessionProjection {
                hall_source: Utf8PathBuf::from(".omp/hooks/pre"),
                config_relative_dest: Utf8PathBuf::from("hooks/pre"),
            },
            SessionProjection {
                hall_source: Utf8PathBuf::from(".omp/extensions"),
                config_relative_dest: Utf8PathBuf::from("extensions"),
            },
        ]
    );
}
