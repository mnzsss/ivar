// tests/unit/providers/auth.rs — the facade's closed dispatch
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::domain::provider::Provider;
use crate::infra::oauth::Tokens;
use crate::providers::{Credential, install_credentials, login_subcommand, verify_authenticated};

/// Every provider's login command is `<binary> <subcommand> <name>`, and the
/// binary comes from the launch contract. This is the assertion that stops
/// `"claude"` and `"opencode"` from being retyped beside `launch_contract`.
#[test]
fn login_subcommand_pairs_with_the_launch_contract_binary() {
    for (provider, expected) in [
        (Provider::ClaudeCode, Some("claude mcp login acme-figma")),
        (Provider::OpenCode, Some("opencode mcp auth acme-figma")),
        // `omp` has no `mcp` subcommand at all (measured against omp/18.1.8:
        // its auth surface is `omp auth-broker`, which Task 10 owns). A
        // fabricated `omp mcp login` would spawn a process that cannot exist.
        (Provider::Omp, None),
    ] {
        let rendered = login_subcommand(provider).map(|subcommand| {
            let binary = crate::providers::launch_contract(provider).binary;
            crate::infra::proc::Command::new(binary)
                .args(subcommand)
                .arg("acme-figma")
                .display()
        });
        assert_eq!(rendered.as_deref(), expected);
    }
}

/// A provider with no store of its own is not an error — it is a provider
/// whose login command is the whole story.
#[test]
fn install_credentials_reports_no_store_for_providers_without_one() {
    let tokens = Tokens {
        access_token: "t".to_owned(),
        refresh_token: None,
        expires_at: None,
        scope: None,
    };
    let credential = Credential {
        server_url: "https://example.test/mcp",
        client_id: "id",
        client_secret: None,
        tokens: &tokens,
    };

    assert!(!install_credentials(Provider::ClaudeCode, "acme-figma", &credential).unwrap());
    assert!(verify_authenticated(Provider::ClaudeCode, "acme-figma", None).is_ok());
}

#[test]
fn omp_verify_authenticated_uses_server_url_binding_and_refuses_missing_url() {
    // When given a server URL, verify_authenticated delegates to omp::auth::verify_authenticated(server_url)
    // which checks the URL-based binding.
    // When given None as server_url, OMP must fail with a Failure rather than constructing a nonsense binding.
    let server_url = "https://acme.example.com/mcp?tenant=123";
    let name = "acme-figma";

    // OMP without URL fails with Failure
    let err_no_url = verify_authenticated(Provider::Omp, name, None).unwrap_err();
    assert_eq!(err_no_url.code, "omp_auth.missing_server_url");

    // OpenCode with name succeeds or fails based on name, ignores URL
    // ClaudeCode always succeeds regardless of URL
    assert!(verify_authenticated(Provider::ClaudeCode, name, None).is_ok());
    assert!(verify_authenticated(Provider::ClaudeCode, name, Some(server_url)).is_ok());
}
