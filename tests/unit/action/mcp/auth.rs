#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::mcp::{resolve_provider, resolve_server};
use crate::domain::mcp::{AuthRequirement, CredentialState, McpAuth, McpServerDef};
use crate::error::Status;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers};
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
        &AuthInput {
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
        &AuthInput {
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
        &AuthInput {
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
        &crate::action::provider::add::AddInput {
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

#[test]
fn provider_order_puts_claude_code_first_if_present() {
    let available = vec![Provider::OpenCode, Provider::ClaudeCode, Provider::Omp];
    let order = provider_order(&available);
    assert_eq!(
        order,
        vec![Provider::ClaudeCode, Provider::OpenCode, Provider::Omp]
    );
}

#[test]
fn provider_order_for_a_hall_listing_t3_never_probes_it_twice() {
    let owners: Vec<Provider> = Providers::new(
        vec![Provider::ClaudeCode, Provider::T3],
        Provider::ClaudeCode,
    )
    .config_owners()
    .collect();
    assert_eq!(provider_order(&owners), vec![Provider::ClaudeCode]);
}

#[test]
fn provider_order_preserves_order_if_claude_code_absent() {
    let available = vec![Provider::OpenCode, Provider::Omp];
    let order = provider_order(&available);
    assert_eq!(order, vec![Provider::OpenCode, Provider::Omp]);
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

// -- recording that a server requires auth --------------------------------------

const PRISDA_URL: &str = "https://mcp.prisda.test/mcp";

/// A claude-code credential store holding (or not) a token for `acme-prisda`.
fn claude_credentials(token: Option<&str>) -> (tempfile::TempDir, camino::Utf8PathBuf) {
    let (dir, root) = crate::test_support::utf8_temp_dir();
    let mcp_oauth = token.map_or_else(
        || serde_json::json!({}),
        |token| {
            serde_json::json!({
                "acme-prisda|638130d5ab3558f4": {
                    "serverName": "acme-prisda",
                    "serverUrl": PRISDA_URL,
                    "accessToken": token,
                    "expiresAt": u64::MAX,
                }
            })
        },
    );
    crate::infra::json::write_canonical(
        &root.join(".credentials.json"),
        &serde_json::json!({ "mcpOAuth": mcp_oauth }),
    )
    .unwrap();
    (dir, root)
}

fn claude_state_under(
    dir: &camino::Utf8Path,
) -> impl Fn(Provider, &str, &str) -> CredentialState + '_ {
    // Reads the fixture the way Claude Code's store is laid out: an entry
    // keyed `<name>|<hash>` that pins the server's URL and holds a token.
    move |_provider, name, url| {
        let text = crate::infra::fs::read_text(&dir.join(".credentials.json"))
            .unwrap()
            .unwrap();
        let file: serde_json::Value = serde_json::from_str(&text).unwrap();
        let prefix = format!("{name}|");
        let held = file["mcpOAuth"]
            .as_object()
            .unwrap()
            .iter()
            .any(|(key, e)| {
                key.starts_with(&prefix)
                    && e["serverUrl"] == url
                    && e["accessToken"].as_str().is_some_and(|t| !t.is_empty())
            });
        if held {
            CredentialState::Authenticated
        } else {
            CredentialState::Missing
        }
    }
}

fn declared_server(root: &camino::Utf8PathBuf) -> McpServerDef {
    let manifest = Manifest::read(&Layout::at(root.clone())).unwrap().unwrap();
    manifest.mcp_servers()[0].clone()
}

fn cimd_server() -> McpServerDef {
    McpServerDef::new("prisda", "http").url(PRISDA_URL)
}

#[test]
fn a_login_that_left_a_credential_marks_a_cimd_server_as_requiring_auth() {
    let (_guard, root) = seeded_hall();
    declare_server(&root, cimd_server());
    let layout = Layout::at(root.clone());
    let (_creds, creds) = claude_credentials(Some("token"));
    let runs = [ok_run(Provider::ClaudeCode)];

    let warning = record_auth_requirement(
        &layout,
        &runs,
        &cimd_server(),
        "acme-prisda",
        claude_state_under(&creds),
    );

    assert!(warning.is_none());
    let saved = declared_server(&root);
    assert_eq!(saved.auth, Some(McpAuth::Oauth));
    assert!(saved.oauth.is_none());
    assert_eq!(saved.auth_requirement(), AuthRequirement::Required);
    let json = std::fs::read_to_string(root.join("ivar.json")).unwrap();
    assert!(json.contains(r#""auth": "oauth""#), "{json}");

    // `status` reads the server through the same credential store, and now
    // reports its token rather than `not-required`.
    let state = crate::action::mcp::status::local_rows(
        "acme",
        &[&saved],
        &[Provider::ClaudeCode],
        &claude_state_under(&creds),
    );
    assert_eq!(state[0].state, CredentialState::Authenticated);
}

#[test]
fn a_status_row_for_a_marked_server_with_no_token_reads_missing_not_required() {
    let (_creds, creds) = claude_credentials(None);
    let marked = cimd_server().auth(McpAuth::Oauth);

    let rows = crate::action::mcp::status::local_rows(
        "acme",
        &[&marked],
        &[Provider::ClaudeCode],
        &claude_state_under(&creds),
    );

    assert_eq!(rows[0].state, CredentialState::Missing);
}

#[test]
fn a_login_that_stored_no_credential_leaves_the_server_unmarked() {
    // `claude mcp login` exits 0 for a server that needs no auth, and the
    // internal flow reports `NoAuthRequired` as authenticated: neither stores
    // a token, so neither is evidence the server demands one.
    let (_guard, root) = seeded_hall();
    declare_server(&root, cimd_server());
    let layout = Layout::at(root.clone());
    let (_creds, creds) = claude_credentials(None);
    let mut run = ok_run(Provider::ClaudeCode);
    run.preregistration = Preregistration::NoAuthRequired;
    let before = std::fs::read_to_string(root.join("ivar.json")).unwrap();

    let warning = record_auth_requirement(
        &layout,
        &[run],
        &cimd_server(),
        "acme-prisda",
        claude_state_under(&creds),
    );

    assert!(warning.is_none());
    assert_eq!(declared_server(&root).auth, None);
    assert_eq!(
        declared_server(&root).auth_requirement(),
        AuthRequirement::NotRequired
    );
    assert_eq!(
        std::fs::read_to_string(root.join("ivar.json")).unwrap(),
        before
    );
}

#[test]
fn a_credential_without_a_successful_run_is_not_evidence() {
    let states = |_p: Provider, _n: &str, _u: &str| CredentialState::Authenticated;
    let failed = [failed_run(Provider::ClaudeCode, "boom")];

    assert!(!has_credential_evidence(
        &failed,
        "acme-prisda",
        PRISDA_URL,
        states
    ));
}

#[test]
fn only_a_present_or_refreshable_credential_is_evidence() {
    let runs = [ok_run(Provider::ClaudeCode)];
    for (state, expected) in [
        (CredentialState::Authenticated, true),
        (CredentialState::Expired, true),
        (CredentialState::Missing, false),
        (CredentialState::Unknown, false),
        (CredentialState::NotRequired, false),
        (CredentialState::NotApplicable, false),
    ] {
        assert_eq!(
            has_credential_evidence(&runs, "acme-prisda", PRISDA_URL, |_, _, _| state),
            expected,
            "{state:?}"
        );
    }
}

#[test]
fn one_provider_holding_a_credential_is_enough_in_an_all_providers_run() {
    let runs = [ok_run(Provider::ClaudeCode), ok_run(Provider::OpenCode)];
    let state = |p: Provider, _: &str, _: &str| {
        if p == Provider::OpenCode {
            CredentialState::Authenticated
        } else {
            CredentialState::Unknown
        }
    };

    assert!(has_credential_evidence(
        &runs,
        "acme-prisda",
        PRISDA_URL,
        state
    ));
}

#[test]
fn a_second_login_does_not_rewrite_a_manifest_that_already_records_auth() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    declare_server(&root, cimd_server());
    assert!(mark_oauth_required(&layout, "prisda").unwrap());
    let before = std::fs::read_to_string(root.join("ivar.json")).unwrap();

    assert!(!mark_oauth_required(&layout, "prisda").unwrap());

    assert_eq!(
        std::fs::read_to_string(root.join("ivar.json")).unwrap(),
        before
    );
}

#[test]
fn a_server_with_a_registered_client_is_not_rewritten() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let registered = cimd_server().oauth(crate::domain::mcp::McpOauth::public("cid"));
    declare_server(&root, registered.clone());
    let (_creds, creds) = claude_credentials(Some("token"));
    let before = std::fs::read_to_string(root.join("ivar.json")).unwrap();

    let warning = record_auth_requirement(
        &layout,
        &[ok_run(Provider::ClaudeCode)],
        &registered,
        "acme-prisda",
        claude_state_under(&creds),
    );

    assert!(warning.is_none());
    assert_eq!(declared_server(&root).auth, None);
    assert_eq!(
        std::fs::read_to_string(root.join("ivar.json")).unwrap(),
        before
    );
}

#[test]
fn marking_re_reads_the_manifest_so_a_fresh_client_registration_survives() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    // `auth()` read the manifest before pre-registration wrote a client; the
    // snapshot it holds is stale, the file is not.
    declare_server(&root, cimd_server());
    let stale = cimd_server();
    declare_server(
        &root,
        cimd_server().oauth(crate::domain::mcp::McpOauth::public("fresh")),
    );
    let (_creds, creds) = claude_credentials(Some("token"));

    let warning = record_auth_requirement(
        &layout,
        &[ok_run(Provider::ClaudeCode)],
        &stale,
        "acme-prisda",
        claude_state_under(&creds),
    );

    assert!(warning.is_none());
    let saved = declared_server(&root);
    assert_eq!(saved.oauth.unwrap().client_id, "fresh");
    assert_eq!(saved.auth, None);
}

#[test]
fn marking_a_server_that_left_the_manifest_is_a_no_op() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());

    assert!(!mark_oauth_required(&layout, "ghost").unwrap());
}
