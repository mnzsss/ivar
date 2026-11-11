#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::super::support::{failed_run, ok_run};
use super::*;
use crate::domain::mcp::CredentialState;

// -- all_providers_report: R-ALL-SEQUENTIAL ordering + R-ALL-PARTIAL --------
//
// `run_provider` itself is not exercised here — both `claude` and `opencode`
// are real, installed binaries on a dev machine, and `proc::inherit` would
// hand either one the real terminal (an interactive, browser-opening OAuth
// flow) if actually invoked. `all_providers_report` is the pure aggregation
// step `auth`'s `--all-providers` branch reduces to after the (unsafe to
// test here) per-provider attempts; feeding it hand-built `ProviderRun`
// values exercises the ordering and partial-failure contract without ever
// spawning a real login command.
#[test]
fn all_providers_report_is_clean_when_every_leg_succeeds() {
    let runs = vec![ok_run(Provider::ClaudeCode), ok_run(Provider::OpenCode)];

    let report = all_providers_report("figma", runs);

    assert!(report.is_clean());
    assert_eq!(
        report
            .value
            .runs
            .iter()
            .map(|r| r.provider)
            .collect::<Vec<_>>(),
        vec![Provider::ClaudeCode, Provider::OpenCode],
        "both providers must be reached, in the order they were given"
    );
}

#[test]
fn all_providers_report_is_unclean_when_any_leg_fails() {
    // The heart of R-ALL-PARTIAL: one success and one failure must not
    // render as a successful run, and both outcomes must still be named.
    let runs = vec![
        ok_run(Provider::ClaudeCode),
        failed_run(
            Provider::OpenCode,
            "`opencode mcp auth acme-figma` exited 1",
        ),
    ];

    let report = all_providers_report("figma", runs);

    assert!(
        !report.is_clean(),
        "one provider succeeding must not make the whole run look clean"
    );
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].subject, "opencode");

    // Both legs are still in the outcome, not just the failed one — the
    // per-provider detail is preserved, never collapsed.
    assert!(report.value.runs[0].authenticated);
    assert_eq!(report.value.runs[0].provider, Provider::ClaudeCode);
    assert!(!report.value.runs[1].authenticated);
    assert_eq!(report.value.runs[1].provider, Provider::OpenCode);

    // The human rendering must name both the success and the failure, not
    // just one of them.
    let mut rendered = Vec::new();
    report.value.write_human(&mut rendered).unwrap();
    let rendered = String::from_utf8(rendered).unwrap();
    assert!(rendered.contains("claude-code"));
    assert!(rendered.contains("opencode"));
    assert!(rendered.contains("Succeeded"));
    assert!(rendered.contains("Failed"));
}

#[test]
fn all_providers_report_single_leg_summary_is_silent() {
    // The single-provider path never sees this function, but the summary
    // line must still not clutter a one-entry report if it ever does.
    let runs = vec![ok_run(Provider::ClaudeCode)];
    let report = all_providers_report("figma", runs);

    let mut rendered = Vec::new();
    report.value.write_human(&mut rendered).unwrap();
    let rendered = String::from_utf8(rendered).unwrap();
    assert!(!rendered.contains("Succeeded:"));
}

#[test]
fn dropped_grants_warns_for_previously_authenticated_provider_now_missing() {
    let runs = vec![
        ProviderRun {
            provider: Provider::ClaudeCode,
            preregistration: Preregistration::NotNeeded,
            command: "claude mcp login".into(),
            auth_method: AuthMethod::ProviderCommand,
            authenticated: true,
            error: None,
        },
        ProviderRun {
            provider: Provider::OpenCode,
            preregistration: Preregistration::NotNeeded,
            command: String::new(),
            auth_method: AuthMethod::InternalOAuthFlow,
            authenticated: true,
            error: None,
        },
    ];

    let mut after = std::collections::BTreeMap::new();
    // ClaudeCode lost its grant after OpenCode ran
    after.insert(Provider::ClaudeCode, CredentialState::Missing);
    after.insert(Provider::OpenCode, CredentialState::Authenticated);

    let warnings = dropped_grants("linear", &runs, &after);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code, "mcp.grant_dropped");
    assert_eq!(warnings[0].subject, Provider::ClaudeCode.id());
    assert!(
        warnings[0]
            .what
            .contains("`linear` lost its claude-code grant after later authorizations")
    );
    assert!(
        warnings[0]
            .what
            .contains("run `ivar mcp auth linear --provider claude-code`")
    );
}

#[test]
fn dropped_grants_empty_when_all_grants_survive() {
    let runs = vec![ProviderRun {
        provider: Provider::ClaudeCode,
        preregistration: Preregistration::NotNeeded,
        command: "claude mcp login".into(),
        auth_method: AuthMethod::ProviderCommand,
        authenticated: true,
        error: None,
    }];

    let mut after = std::collections::BTreeMap::new();
    after.insert(Provider::ClaudeCode, CredentialState::Authenticated);

    let warnings = dropped_grants("linear", &runs, &after);
    assert!(warnings.is_empty());
}
