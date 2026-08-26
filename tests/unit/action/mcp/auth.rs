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
        McpServerDef::new("linear-gaio", "stdio").command("linear-mcp"),
    );
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let server = resolve_server(&manifest, "linear-gaio").unwrap();
    assert_eq!(server.name, "linear-gaio");
}

#[test]
fn resolve_server_lists_the_declared_names_when_absent() {
    let (_guard, root) = seeded_hall();
    declare_server(
        &root,
        McpServerDef::new("linear-gaio", "stdio").command("linear-mcp"),
    );
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let failure = resolve_server(&manifest, "figma-gaio").unwrap_err();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "mcp.server_not_found");
    assert!(failure.expected.unwrap().contains("linear-gaio"));
}

#[test]
fn resolve_server_names_the_empty_case_when_none_are_declared() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();

    let failure = resolve_server(&manifest, "figma-gaio").unwrap_err();
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

// -- preregister_if_needed: the branches that never touch the filesystem --
//
// Every branch below returns before ever reading `layout` or `manifest` (a
// different provider, no `url`, a host off the allowlist, or a server that
// already carries `oauth`), so a `seeded_hall()` is enough scaffolding —
// none of these tests reach the network or rewrite `ivar.json`.

#[test]
fn preregistration_not_needed_for_claude_code() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let server = McpServerDef::new("figma-gaio", "sse").url("https://mcp.figma.com/mcp");

    let result = preregister_if_needed(&layout, &manifest, Provider::ClaudeCode, &server).unwrap();
    assert!(matches!(result.report, Preregistration::NotNeeded));
}

#[test]
fn preregistration_not_needed_without_a_url() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let server = McpServerDef::new("linear-gaio", "stdio").command("linear-mcp");

    let result = preregister_if_needed(&layout, &manifest, Provider::OpenCode, &server).unwrap();
    assert!(matches!(result.report, Preregistration::NotNeeded));
}

#[test]
fn preregistration_not_needed_for_a_host_off_the_allowlist() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let server = McpServerDef::new("linear-gaio", "sse").url("https://mcp.linear.app/mcp");

    let result = preregister_if_needed(&layout, &manifest, Provider::OpenCode, &server).unwrap();
    assert!(matches!(result.report, Preregistration::NotNeeded));
}

/// R-IDEMPOTENT, the manifest half: a server whose entry already carries
/// `oauth` is skipped outright, never re-registered — no network call, no
/// rewrite of `ivar.json`.
#[test]
fn preregistration_skipped_when_the_manifest_already_carries_oauth() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    // `CARGO_MANIFEST_DIR` is a variable cargo always sets on the test
    // process itself — used here purely as "a variable guaranteed to be
    // set", to exercise the present-and-usable branch without mutating the
    // process environment (`unsafe_code` is denied in this crate, so
    // `std::env::set_var` is not an option).
    let server = McpServerDef::new("figma-gaio", "sse")
        .url("https://mcp.figma.com/mcp")
        .oauth(McpOauth::new("existing-client", "CARGO_MANIFEST_DIR"));

    let result = preregister_if_needed(&layout, &manifest, Provider::OpenCode, &server).unwrap();
    assert!(matches!(result.report, Preregistration::Skipped));
}

/// Defect fix, related improvement (`R-ERRORS`): on the `Skipped` path
/// `ivar` never held this run's secret, so a missing export must fail
/// early, naming the variable — rather than dispatch into OpenCode's
/// confusing `client_secret_basic authentication requires a client_secret`.
#[test]
fn preregistration_skipped_path_fails_naming_the_variable_when_it_is_unset() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let server = McpServerDef::new("figma-gaio", "sse")
        .url("https://mcp.figma.com/mcp")
        .oauth(McpOauth::new(
            "existing-client",
            "IVAR_MCP_AUTH_TEST_DOES_NOT_EXIST_UNSET",
        ));

    let failure =
        preregister_if_needed(&layout, &manifest, Provider::OpenCode, &server).unwrap_err();

    assert_eq!(failure.code, "mcp.missing_client_secret_env");
    assert!(
        failure
            .what
            .contains("IVAR_MCP_AUTH_TEST_DOES_NOT_EXIST_UNSET")
    );
}

// -- secret_env_var: the one place the export variable name is built -------

#[test]
fn secret_env_var_uppercases_and_folds_non_alphanumerics() {
    assert_eq!(secret_env_var("figma-gaio"), "IVAR_MCP_FIGMA_GAIO_SECRET");
    assert_eq!(secret_env_var("linear"), "IVAR_MCP_LINEAR_SECRET");
}

// -- print_secret_export ----------------------------------------------------

/// A smoke test, not a content check: stderr is always writable in a test
/// process, and capturing it is not worth the machinery for a two-line
/// function. What actually matters — the secret never touching `ivar.json`,
/// any materialised file, or `AuthOutcome` — is enforced by construction
/// (see the module doc comment's "The secret handoff" section):
/// `Preregistration::Registered` has no field that could hold one, and
/// `all_providers_report`'s tests below never construct one that does.
#[test]
fn print_secret_export_succeeds_against_real_stderr() {
    print_secret_export("IVAR_MCP_FIGMA_GAIO_SECRET", "shh").unwrap();
}

// -- host_of --------------------------------------------------------------

#[test]
fn host_of_strips_scheme_path_query_and_fragment() {
    assert_eq!(
        host_of("https://mcp.figma.com/mcp?x=1#frag"),
        Some("mcp.figma.com")
    );
}

#[test]
fn host_of_strips_port_and_userinfo() {
    assert_eq!(
        host_of("https://user:pass@mcp.figma.com:443/mcp"),
        Some("mcp.figma.com")
    );
}

#[test]
fn host_of_works_without_a_scheme() {
    assert_eq!(host_of("mcp.figma.com/mcp"), Some("mcp.figma.com"));
}

// -- auth_command -----------------------------------------------------------

#[test]
fn auth_command_is_claude_mcp_login_for_claude_code() {
    let command = auth_command(Harness::ClaudeCode, "figma-gaio", None);
    assert_eq!(command.display(), "claude mcp login figma-gaio");
}

#[test]
fn auth_command_is_opencode_mcp_auth_for_opencode() {
    let command = auth_command(Harness::OpenCode, "figma-gaio", None);
    assert_eq!(command.display(), "opencode mcp auth figma-gaio");
}

#[test]
fn auth_command_carries_no_env_override_without_a_fresh_secret() {
    let command = auth_command(Harness::OpenCode, "figma-gaio", None);
    assert!(command.envs().is_empty());
}

/// Defect fix (`R-SECRET-HANDOFF`): a fresh registration's secret must reach
/// the dispatched child's own environment — the operator cannot have
/// exported it yet on the run that just minted it. Asserted on the built
/// `Command` only; nothing here spawns a process.
#[test]
fn auth_command_puts_a_fresh_registrations_secret_into_the_childs_environment() {
    let fresh = (
        "IVAR_MCP_FIGMA_GAIO_SECRET".to_owned(),
        "top-secret".to_owned(),
    );

    let command = auth_command(Harness::OpenCode, "figma-gaio", Some(&fresh));

    assert_eq!(
        command.envs(),
        &[(
            "IVAR_MCP_FIGMA_GAIO_SECRET".to_owned(),
            "top-secret".to_owned()
        )]
    );
    // The secret must never show up in the human-readable command line.
    assert!(!command.display().contains("top-secret"));
}

// -- login_failed -----------------------------------------------------------

#[test]
fn login_failed_names_the_exit_code() {
    let failure = login_failed("claude mcp login figma-gaio", Some(1));
    assert_eq!(failure.code, "mcp.auth_failed");
    assert!(failure.what.contains("exited 1"));
}

#[test]
fn login_failed_names_a_signal_death() {
    let failure = login_failed("claude mcp login figma-gaio", None);
    assert!(failure.what.contains("killed by a signal"));
}

// -- auth(): the failure paths that never reach dispatch ---------------------

#[test]
fn auth_refuses_an_unknown_server_before_any_dispatch() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let failure = auth(
        &ctx,
        AuthInput {
            server: "figma-gaio".to_owned(),
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
        McpServerDef::new("linear-gaio", "stdio").command("linear-mcp"),
    );
    let ctx = Ctx::new(root.clone());

    let failure = auth(
        &ctx,
        AuthInput {
            server: "linear-gaio".to_owned(),
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
            server: "figma-gaio".to_owned(),
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
        command: format!("{} mcp auth figma-gaio", provider.id()),
        authenticated: true,
        error: None,
    }
}

fn failed_run(provider: Provider, error: &str) -> ProviderRun {
    ProviderRun {
        provider,
        preregistration: Preregistration::NotNeeded,
        command: format!("{} mcp auth figma-gaio", provider.id()),
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

    let report = all_providers_report("figma-gaio", runs);

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
            "`opencode mcp auth figma-gaio` exited 1",
        ),
    ];

    let report = all_providers_report("figma-gaio", runs);

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
    let report = all_providers_report("figma-gaio", runs);

    let mut rendered = Vec::new();
    report.value.write_human(&mut rendered).unwrap();
    let rendered = String::from_utf8(rendered).unwrap();
    assert!(!rendered.contains("Succeeded:"));
}
