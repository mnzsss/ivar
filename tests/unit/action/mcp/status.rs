use std::collections::BTreeMap;

use crate::action::mcp::status::{
    apply_live, local_rows, status_report, StateSource, StatusOutcome, StatusRow,
};
use crate::domain::mcp::{CredentialState, McpOauth, McpServerDef};
use crate::domain::provider::Provider;

#[test]
fn local_rows_builds_matrix_respecting_auth_requirement() {
    let hall = "acme";
    let local_server = McpServerDef::new("docs", "local").command("npx");
    let http_no_oauth = McpServerDef::new("sentry", "http").url("https://sentry.io/mcp");
    let http_oauth = McpServerDef::new("figma", "http")
        .url("https://mcp.figma.com/mcp")
        .oauth(McpOauth::public("cid"));

    let servers = vec![&local_server, &http_no_oauth, &http_oauth];
    let providers = vec![Provider::ClaudeCode, Provider::OpenCode];

    let mock_state = |p: Provider, name: &str, _url: &str| -> CredentialState {
        if name == "acme-figma" && p == Provider::ClaudeCode {
            CredentialState::Authenticated
        } else {
            CredentialState::Missing
        }
    };

    let rows = local_rows(hall, &servers, &providers, &mock_state);

    // docs: local -> NotApplicable for all providers
    assert_eq!(rows[0].server, "docs");
    assert_eq!(rows[0].provider, Provider::ClaudeCode);
    assert_eq!(rows[0].state, CredentialState::NotApplicable);
    assert_eq!(rows[0].source, StateSource::Local);

    assert_eq!(rows[1].server, "docs");
    assert_eq!(rows[1].provider, Provider::OpenCode);
    assert_eq!(rows[1].state, CredentialState::NotApplicable);

    // sentry: http w/o oauth -> NotRequired
    assert_eq!(rows[2].server, "sentry");
    assert_eq!(rows[2].provider, Provider::ClaudeCode);
    assert_eq!(rows[2].state, CredentialState::NotRequired);

    // figma: http w/ oauth -> uses mock_state
    assert_eq!(rows[4].server, "figma");
    assert_eq!(rows[4].provider, Provider::ClaudeCode);
    assert_eq!(rows[4].state, CredentialState::Authenticated);

    assert_eq!(rows[5].server, "figma");
    assert_eq!(rows[5].provider, Provider::OpenCode);
    assert_eq!(rows[5].state, CredentialState::Missing);
}

#[test]
fn apply_live_updates_eligible_rows() {
    let mut rows = vec![
        StatusRow {
            server: "docs".to_owned(),
            materialised_name: "acme-docs".to_owned(),
            provider: Provider::ClaudeCode,
            state: CredentialState::NotApplicable,
            source: StateSource::Local,
        },
        StatusRow {
            server: "figma".to_owned(),
            materialised_name: "acme-figma".to_owned(),
            provider: Provider::ClaudeCode,
            state: CredentialState::Missing,
            source: StateSource::Local,
        },
    ];

    let mut live_map = BTreeMap::new();
    let mut claude_entries = BTreeMap::new();
    claude_entries.insert("acme-figma".to_owned(), CredentialState::Authenticated);
    live_map.insert(Provider::ClaudeCode, claude_entries);

    apply_live(&mut rows, &live_map);

    // docs stays Local / NotApplicable
    assert_eq!(rows[0].source, StateSource::Local);
    assert_eq!(rows[0].state, CredentialState::NotApplicable);

    // figma becomes Live / Authenticated
    assert_eq!(rows[1].source, StateSource::Live);
    assert_eq!(rows[1].state, CredentialState::Authenticated);
}

#[test]
fn status_report_generates_warnings_for_attention_states() {
    let rows = vec![
        StatusRow {
            server: "figma".to_owned(),
            materialised_name: "acme-figma".to_owned(),
            provider: Provider::ClaudeCode,
            state: CredentialState::Missing,
            source: StateSource::Local,
        },
        StatusRow {
            server: "linear".to_owned(),
            materialised_name: "acme-linear".to_owned(),
            provider: Provider::OpenCode,
            state: CredentialState::Authenticated,
            source: StateSource::Local,
        },
    ];

    let report = status_report(rows);
    assert!(!report.is_clean());
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].code, "mcp.auth_needs_attention");
    assert_eq!(report.warnings[0].subject.as_str(), "figma/claude-code");
    assert!(report.warnings[0].what.contains("ivar mcp auth figma --provider claude-code"));
}

#[test]
fn no_token_or_secret_appears_in_serialized_or_debug_output() {
    let row = StatusRow {
        server: "figma".to_owned(),
        materialised_name: "acme-figma".to_owned(),
        provider: Provider::ClaudeCode,
        state: CredentialState::Authenticated,
        source: StateSource::Local,
    };
    let outcome = StatusOutcome { rows: vec![row] };

    let json_str = serde_json::to_string(&outcome).unwrap();
    let debug_str = format!("{outcome:?}");

    // Verify no secret substrings or token fields exist in data structures
    assert!(!json_str.contains("secret"));
    assert!(!json_str.contains("token"));
    assert!(!debug_str.contains("secret"));
}
