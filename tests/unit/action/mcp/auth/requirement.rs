#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::super::Preregistration;
use super::super::support::{declare_server, failed_run, ok_run};
use super::*;
use crate::domain::mcp::AuthRequirement;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

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
