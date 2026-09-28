#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;

use crate::action::mcp::status::{
    StateSource, StatusOutcome, StatusRow, apply_live, local_rows, status_report,
};
use crate::domain::mcp::{CredentialState, McpOauth, McpServerDef};
use crate::domain::provider::Provider;
use crate::error::WriteHuman;

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
    let [r0, r1, r2, _r3, r4, r5] = rows.as_slice() else {
        panic!("expected 6 rows, got {}", rows.len());
    };

    // docs: local -> NotApplicable for all providers
    assert_eq!(r0.server, "docs");
    assert_eq!(r0.provider, Provider::ClaudeCode);
    assert_eq!(r0.state, CredentialState::NotApplicable);
    assert_eq!(r0.source, StateSource::Local);

    assert_eq!(r1.server, "docs");
    assert_eq!(r1.provider, Provider::OpenCode);
    assert_eq!(r1.state, CredentialState::NotApplicable);

    // sentry: http w/o oauth -> NotRequired
    assert_eq!(r2.server, "sentry");
    assert_eq!(r2.provider, Provider::ClaudeCode);
    assert_eq!(r2.state, CredentialState::NotRequired);

    // figma: http w/ oauth -> uses mock_state
    assert_eq!(r4.server, "figma");
    assert_eq!(r4.provider, Provider::ClaudeCode);
    assert_eq!(r4.state, CredentialState::Authenticated);

    assert_eq!(r5.server, "figma");
    assert_eq!(r5.provider, Provider::OpenCode);
    assert_eq!(r5.state, CredentialState::Missing);
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

    let [r0, r1] = rows.as_slice() else {
        panic!("expected 2 rows, got {}", rows.len());
    };

    // docs stays Local / NotApplicable
    assert_eq!(r0.source, StateSource::Local);
    assert_eq!(r0.state, CredentialState::NotApplicable);

    // figma becomes Live / Authenticated
    assert_eq!(r1.source, StateSource::Live);
    assert_eq!(r1.state, CredentialState::Authenticated);
}

#[test]
fn apply_live_retains_local_when_provider_absent_and_marks_server_absent_as_unknown() {
    let mut rows = vec![
        StatusRow {
            server: "figma".to_owned(),
            materialised_name: "acme-figma".to_owned(),
            provider: Provider::ClaudeCode,
            state: CredentialState::Authenticated,
            source: StateSource::Local,
        },
        StatusRow {
            server: "omp-server".to_owned(),
            materialised_name: "acme-omp-server".to_owned(),
            provider: Provider::Omp,
            state: CredentialState::Authenticated,
            source: StateSource::Local,
        },
    ];

    let mut live_map = BTreeMap::new();
    live_map.insert(Provider::ClaudeCode, BTreeMap::new());

    apply_live(&mut rows, &live_map);

    let [r0, r1] = rows.as_slice() else {
        panic!("expected 2 rows, got {}", rows.len());
    };

    assert_eq!(r0.source, StateSource::Live);
    assert_eq!(r0.state, CredentialState::Unknown);

    assert_eq!(r1.source, StateSource::Local);
    assert_eq!(r1.state, CredentialState::Authenticated);
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
    let [warning] = report.warnings.as_slice() else {
        panic!("expected 1 warning, got {:?}", report.warnings);
    };
    assert_eq!(warning.code, "mcp.auth_needs_attention");
    assert_eq!(warning.subject.as_str(), "figma/claude-code");
    assert_eq!(
        warning.what.as_str(),
        "`figma` is missing for claude-code; run `ivar mcp auth figma --provider claude-code`"
    );
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

    let json_str = serde_json::to_string(&outcome).expect("serialize outcome to json");
    let debug_str = format!("{outcome:?}");

    // Verify no secret substrings or token fields exist in data structures
    assert!(!json_str.contains("secret"));
    assert!(!json_str.contains("token"));
    assert!(!debug_str.contains("secret"));
}

#[test]
fn table_rendering_formats_kebab_case_and_aligns_long_server_names() {
    let rows = vec![
        StatusRow {
            server: "cloudflare-observability".to_owned(),
            materialised_name: "acme-cloudflare-observability".to_owned(),
            provider: Provider::ClaudeCode,
            state: CredentialState::NotApplicable,
            source: StateSource::Local,
        },
        StatusRow {
            server: "figma".to_owned(),
            materialised_name: "acme-figma".to_owned(),
            provider: Provider::OpenCode,
            state: CredentialState::Authenticated,
            source: StateSource::Live,
        },
    ];

    let outcome = StatusOutcome { rows };
    let mut buf = Vec::new();
    let res = outcome.write_human(&mut buf);
    assert!(res.is_ok());

    let output = String::from_utf8(buf).unwrap_or_default();
    let lines: Vec<&str> = output.lines().collect();
    let [header, row1, row2] = lines.as_slice() else {
        panic!("expected 3 lines, got {:?}", lines);
    };

    // Header should be formatted
    assert!(header.starts_with("SERVER"));
    assert!(header.contains("PROVIDER"));
    assert!(header.contains("STATE (SOURCE)"));

    // Check column alignment: provider and state start at identical offsets across rows
    let p1_idx = row1.find("claude-code").unwrap_or(0);
    let p2_idx = row2.find("opencode").unwrap_or(0);
    assert_eq!(p1_idx, p2_idx);
    assert!(p1_idx > "cloudflare-observability".len());

    let s1_idx = row1.find("not-applicable").unwrap_or(0);
    let s2_idx = row2.find("authenticated (live)").unwrap_or(0);
    assert_eq!(s1_idx, s2_idx);
}
