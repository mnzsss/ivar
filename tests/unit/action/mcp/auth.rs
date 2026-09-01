#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::mcp::McpServerDef;
use crate::error::Status;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

/// Declare one MCP server on the seeded hall's manifest and write it back.
fn declare_server(root: &camino::Utf8PathBuf, server: McpServerDef) {
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let updated = manifest.with_mcp_servers(vec![server]).unwrap();
    Manifest::write(&layout, &updated).unwrap();
}

// -- resolve_server -----------------------------------------------------

#[test]
fn resolve_server_finds_a_declared_server_by_name() {
    let (_guard, root) = seeded_hall();
    declare_server(
        &root,
        McpServerDef::new("linear", "local").command("linear-mcp"),

    );
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let server = resolve_server(&manifest, "linear").unwrap();
    assert_eq!(server.name, "linear");
}

#[test]
fn resolve_server_lists_the_declared_names_when_absent() {
    let (_guard, root) = seeded_hall();
    declare_server(
        &root,
        McpServerDef::new("linear", "local").command("linear-mcp"),

    );
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let failure = resolve_server(&manifest, "figma").unwrap_err();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "mcp.server_not_found");
    assert!(failure.expected.unwrap().contains("linear"));
}

#[test]
fn resolve_server_names_the_empty_case_when_none_are_declared() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let failure = resolve_server(&manifest, "figma").unwrap_err();
    assert!(failure.expected.unwrap().contains("no servers declared"));
}

// -- resolve_provider -----------------------------------------------------

#[test]
fn resolve_provider_defaults_to_the_hall_default() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    assert_eq!(
        resolve_provider(&manifest, None).unwrap(),
        Provider::ClaudeCode
    );
}

#[test]
fn resolve_provider_honours_an_explicit_override() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    assert_eq!(
        resolve_provider(&manifest, Some("opencode")).unwrap(),
        Provider::OpenCode
    );
}

#[test]
fn resolve_provider_rejects_an_unknown_id() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let failure = resolve_provider(&manifest, Some("bogus")).unwrap_err();
    assert_eq!(failure.status, Status::Blocked);
}

// -- auth(): the failure paths that never reach dispatch ---------------------

#[test]
fn auth_refuses_an_unknown_server_before_any_dispatch() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let failure = auth(
        &ctx,
        AuthInput {
            server: "figma".to_owned(),
            provider: None,
            all_providers: false,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "mcp.server_not_found");
}

#[test]
fn auth_refuses_an_unknown_provider_before_any_dispatch() {
    let (_guard, root) = seeded_hall();
    declare_server(
        &root,
        McpServerDef::new("linear", "local").command("linear-mcp"),

    );
    let ctx = Ctx::new(root.clone());

    let failure = auth(
        &ctx,
        AuthInput {
            server: "linear".to_owned(),
            provider: Some("bogus".to_owned()),
            all_providers: false,
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
}

#[test]
fn auth_refuses_an_unknown_server_for_all_providers_too() {
    // `--all-providers` still resolves the server before it ever touches the
    // provider loop — an unknown name is not "0 of N providers succeeded",
    // it is the same refusal as the single-provider form.
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let failure = auth(
        &ctx,
        AuthInput {
            server: "figma".to_owned(),
            provider: None,
            all_providers: true,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "mcp.server_not_found");
}

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

fn ok_run(provider: Provider) -> ProviderRun {
    ProviderRun {
        provider,
        preregistration: Preregistration::NotNeeded,
        command: format!("{} mcp auth acme-figma", provider.id()),
        auth_method: AuthMethod::ProviderCommand,
        authenticated: true,
        error: None,
    }
}

fn failed_run(provider: Provider, error: &str) -> ProviderRun {
    ProviderRun {
        provider,
        preregistration: Preregistration::NotNeeded,
        command: format!("{} mcp auth acme-figma", provider.id()),
        auth_method: AuthMethod::ProviderCommand,
        authenticated: false,
        error: Some(error.to_owned()),
    }
}

#[test]
fn all_providers_hall_lists_available_providers_in_declared_order() {
    // Grounds the ordering claim in the actual manifest a two-provider hall
    // produces — `auth`'s `.map` over this exact sequence is what makes
    // `--all-providers` "sequential, in order" rather than an assumption.
    let (_guard, root) = seeded_hall();
    let ctx = crate::action::Ctx::new(root.clone());
    crate::action::provider::add::add(
        &ctx,
        crate::action::provider::add::AddInput {
            name: "opencode".to_owned(),
        },
    )
    .unwrap();

    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    assert_eq!(
        manifest.providers().available(),
        &[Provider::ClaudeCode, Provider::OpenCode]
    );
}

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
