//! Internal OAuth flow for OpenCode + Figma, owned entirely by Ivar.
//!
//! When the provider is OpenCode and the server's host is on Figma's
//! pre-registration allowlist, Ivar performs the full authorization-code
//! + PKCE flow itself instead of delegating to `opencode mcp auth`
//! (`R-FIGMA-FLOW`). This module orchestrates the steps:
//!
//! 1. **Conflict check** — `opencode_auth::has_entry(materialised_name)`
//!    before anything else (`R-CONFLICT`).
//! 2. **Pre-registration** — reuse [`preregister_if_needed`] for
//!    client_id / client_secret.
//! 3. **Endpoint discovery** — [`figma::discover_oauth_endpoints`].
//! 4. **PKCE + state** — [`oauth::pkce_pair`] and [`oauth::state`].
//! 5. **Listener** — bind `127.0.0.1:19876` before printing the URL.
//! 6. **Print URL** — authorization URL for manual browser opening.
//! 7. **Wait for callback** — validate state, receive code.
//! 8. **Code exchange** — [`oauth::exchange_code`].
//! 9. **Persist** — [`opencode_auth::write_entry`].
//! 10. **Verify** — [`opencode_auth::has_tokens`].
//!
//! Failure at any step before 9 leaves the credential store unchanged
//! (`R-ATOMIC`). `Ctrl+C` terminates the process; the OS releases the
//! loopback socket; nothing partial is written.
//!
//! # Module boundaries
//!
//! `action` may import `domain`, `infra`, `harness`, and `store`. This
//! module reaches into all four layers for the orchestration steps.

use std::io::{self, Write};
use std::time::Duration;

use crate::domain::mcp::McpServerDef;
use crate::error::{Failure, FixAction};
use crate::harness::opencode_auth::{self, ClientInfo, Entry};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::preregister::preregister_if_needed;
use super::{AuthMethod, Preregistration, ProviderRun};

use crate::infra::figma;
use crate::infra::fs;
use crate::infra::http_callback::CallbackServer;
use crate::infra::oauth;

/// How long to wait for the OAuth callback before giving up.
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(120);

/// The redirect URI — must match the one registered with Figma and the one
/// `opencode.json`'s `oauth.redirectUri` declares.
const REDIRECT_URI: &str = "http://127.0.0.1:19876/callback";

/// The label used in `ProviderRun::command` for the internal flow, since
/// there is no child process command to display.
const INTERNAL_FLOW_LABEL: &str = "ivar oauth";

/// Run the full internal OAuth flow for OpenCode + Figma.
///
/// Returns a [`ProviderRun`] with the result. On success, the credential
/// store contains a complete entry and `has_tokens` is true.
///
/// This is the public entry point called from [`dispatch`](super::dispatch).
#[allow(dead_code)]
pub(super) fn run_internal_flow(
    layout: &Layout,
    manifest: &Manifest,
    server: &McpServerDef,
    materialised_name: &str,
) -> ProviderRun {
    let provider = crate::domain::provider::Provider::OpenCode;

    match run_internal_flow_inner(layout, manifest, server, materialised_name) {
        Ok(run) => run,
        Err(failure) => ProviderRun {
            provider,
            preregistration: Preregistration::NotNeeded,
            auth_method: AuthMethod::InternalOAuthFlow,
            command: INTERNAL_FLOW_LABEL.to_owned(),
            authenticated: false,
            error: Some(failure.what),
        },
    }
}

pub(super) fn run_internal_flow_inner(
    layout: &Layout,
    manifest: &Manifest,
    server: &McpServerDef,
    materialised_name: &str,
) -> Result<ProviderRun, Failure> {
    let provider = crate::domain::provider::Provider::OpenCode;

    // Step 1: Conflict check — before any registration or network request.
    if opencode_auth::has_entry(materialised_name)? {
        return Err(conflict_failure(materialised_name, layout));
    }

    // Step 2: Pre-register (or skip if manifest already has OAuth).
    let preregistered =
        preregister_if_needed(layout, manifest, provider, server, materialised_name)?;
    let preregistration = preregistered.report.clone();

    // Extract client_id and secret for the flow.
    let client_id = preregistered.client_id.ok_or_else(|| {
        Failure::blocked(
            "figma_oauth.no_client_id",
            "internal OAuth flow requires a client_id from pre-registration",
        )
    })?;
    let client_secret = match preregistered.secret {
        Some((_var, secret)) => secret,
        None => {
            return Err(Failure::blocked(
                "figma_oauth.no_client_secret",
                "internal OAuth flow requires a client secret from pre-registration",
            ));
        }
    };

    // Step 3: Discover OAuth endpoints from the MCP server.
    let server_url = server.url.as_deref().ok_or_else(|| {
        Failure::blocked(
            "figma_oauth.no_server_url",
            "internal OAuth flow requires a server URL for endpoint discovery",
        )
    })?;
    let endpoints = figma::discover_oauth_endpoints(server_url)?;

    // Step 4: Generate PKCE pair and state.
    let (verifier, challenge) = oauth::pkce_pair();
    let state = oauth::state();

    // Step 5: Bind the callback listener BEFORE printing the URL, to avoid
    // a race where the browser redirects before the listener is ready.
    let listener = CallbackServer::bind(&state.0, CALLBACK_TIMEOUT)?;

    // Step 6: Build and print the authorization URL for manual opening.
    let auth_url = oauth::authorize_url(
        &endpoints.authorization_endpoint,
        &client_id,
        REDIRECT_URI,
        &state,
        &challenge,
        endpoints.resource.as_deref(),
        endpoints
            .scopes_supported
            .as_deref()
            .and_then(|s| s.first())
            .map(|s| s.as_str()),
    );

    // Write to stderr — this runs before the Outcome renderer, so stdout
    // is not yet claimed. stderr is the same seam `confirm` and `progress`
    // use for interactive terminal output.
    let _ = writeln!(
        io::stderr().lock(),
        "Open this URL to authenticate:\n\n  {auth_url}\n"
    );

    // Step 7: Wait for the callback (validates state internally).
    let code = listener.wait()?;

    // Step 8: Exchange the authorization code for tokens.
    let tokens = oauth::exchange_code(
        &endpoints.token_endpoint,
        &code.0,
        REDIRECT_URI,
        &verifier,
        &client_id,
        &client_secret,
    )?;

    // Step 9: Build and persist the credential store entry.
    let entry = Entry {
        server_url: server_url.to_owned(),
        client_info: ClientInfo {
            client_id,
            client_secret: Some(client_secret),
            client_secret_expires_at: None,
        },
        tokens,
    };
    opencode_auth::write_entry(materialised_name, &entry)?;

    // Step 10: Verify the entry was written correctly.
    if !opencode_auth::has_tokens(materialised_name)? {
        return Err(Failure::failed(
            "figma_oauth.verify_failed",
            "token exchange succeeded but has_tokens returned false after write",
        )
        .expected("has_tokens to return true after write_entry")
        .actual("has_tokens returned false"));
    }

    Ok(ProviderRun {
        provider,
        preregistration,
        auth_method: AuthMethod::InternalOAuthFlow,
        command: INTERNAL_FLOW_LABEL.to_owned(),
        authenticated: true,
        error: None,
    })
}

/// Build the conflict failure, naming the server and the store path.
fn conflict_failure(materialised_name: &str, _layout: &Layout) -> Failure {
    let path = fs::data_dir()
        .map(|d| d.join("opencode").join("mcp-auth.json"))
        .map(|p| p.to_string())
        .unwrap_or_else(|_| "OpenCode's mcp-auth.json".to_owned());

    Failure::blocked(
        "figma_oauth.conflict",
        format!("the credential store already has an entry for \"{materialised_name}\""),
    )
    .expected("no existing entry for this server name in the credential store")
    .actual(format!("an entry already exists at {path}"))
    .fix(FixAction::unsafe_(
        "figma_oauth.remove_entry",
        format!(
            "Remove the \"{materialised_name}\" entry from the credential store \
             explicitly before re-authenticating: delete the entry from {path}."
        ),
    ))
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/mcp/auth/figma_oauth.rs"]
mod tests;
