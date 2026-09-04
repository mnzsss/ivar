//! Internal OAuth flow for OpenCode + Figma, owned entirely by Ivar.
//!
//! When the provider is OpenCode and the server's host is on Figma's
//! pre-registration allowlist, Ivar performs the full authorization-code
//! + PKCE flow itself instead of delegating to `opencode mcp auth`.
//!
//! This module orchestrates the steps:
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

use super::preregister::{Preregistered, preregister_if_needed};
use super::{AuthMethod, Preregistration, ProviderRun};

use crate::infra::figma::{self, OAuthEndpoints};
use crate::infra::fs;
use crate::infra::http_callback::{AuthorizationCode, CallbackServer, OAUTH_REDIRECT_URI};
use crate::infra::oauth::{self, AuthMode, Tokens};

/// How long to wait for the OAuth callback before giving up.
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(120);

/// The redirect URI — must match the one registered with Figma and the one
/// `opencode.json`'s `oauth.redirectUri` declares, which is exactly why it is
/// one shared constant beside the listener rather than a local copy.
const REDIRECT_URI: &str = OAUTH_REDIRECT_URI;

/// The label used in `ProviderRun::command` for the internal flow, since
/// there is no child process command to display.
const INTERNAL_FLOW_LABEL: &str = "ivar oauth";

pub(super) trait FlowOps {
    fn check_conflict(&self, name: &str) -> Result<bool, Failure>;
    fn preregister(&self, server: &McpServerDef, name: &str) -> Result<Preregistered, Failure>;
    fn discover(&self, url: &str) -> Result<OAuthEndpoints, Failure>;
    fn bind(&self, state: &str) -> Result<CallbackServer, Failure>;
    fn output_url(&self, url: &str);
    fn wait_code(&self, listener: CallbackServer) -> Result<AuthorizationCode, Failure>;
    #[allow(clippy::too_many_arguments)]
    fn exchange(
        &self,
        endpoint: &str,
        code: &str,
        verifier: &str,
        id: &str,
        secret: Option<&str>,
        mode: AuthMode,
        resource: Option<&str>,
    ) -> Result<Tokens, Failure>;
    fn write(&self, name: &str, entry: &Entry) -> Result<(), Failure>;
    fn verify(&self, name: &str) -> Result<bool, Failure>;
}

struct RealFlowOps {
    layout: Layout,
    manifest: Manifest,
    provider: crate::domain::provider::Provider,
}

impl FlowOps for RealFlowOps {
    fn check_conflict(&self, name: &str) -> Result<bool, Failure> {
        opencode_auth::has_entry(name)
    }
    fn preregister(&self, server: &McpServerDef, name: &str) -> Result<Preregistered, Failure> {
        preregister_if_needed(&self.layout, &self.manifest, self.provider, server, name)
    }
    fn discover(&self, url: &str) -> Result<OAuthEndpoints, Failure> {
        figma::discover_oauth_endpoints(url)
    }
    fn bind(&self, state: &str) -> Result<CallbackServer, Failure> {
        CallbackServer::bind(state, CALLBACK_TIMEOUT)
    }
    fn output_url(&self, url: &str) {
        let _ = writeln!(
            io::stderr().lock(),
            "Open this URL to authenticate:\n\n  {url}\n"
        );
    }
    fn wait_code(&self, listener: CallbackServer) -> Result<AuthorizationCode, Failure> {
        listener.wait()
    }
    fn exchange(
        &self,
        endpoint: &str,
        code: &str,
        verifier: &str,
        id: &str,
        secret: Option<&str>,
        mode: AuthMode,
        resource: Option<&str>,
    ) -> Result<Tokens, Failure> {
        oauth::exchange_code(
            endpoint,
            code,
            REDIRECT_URI,
            &oauth::CodeVerifier(verifier.to_owned()),
            id,
            secret,
            mode,
            resource,
        )
    }
    fn write(&self, name: &str, entry: &Entry) -> Result<(), Failure> {
        opencode_auth::write_entry(name, entry)
    }
    fn verify(&self, name: &str) -> Result<bool, Failure> {
        opencode_auth::has_tokens(name)
    }
}

/// Run the full internal OAuth flow for OpenCode + Figma.
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
    let ops = RealFlowOps {
        layout: layout.clone(),
        manifest: manifest.clone(),
        provider: crate::domain::provider::Provider::OpenCode,
    };
    run_internal_flow_pipeline(&ops, server, materialised_name)
}

pub(super) fn run_internal_flow_pipeline(
    ops: &dyn FlowOps,
    server: &McpServerDef,
    materialised_name: &str,
) -> Result<ProviderRun, Failure> {
    let provider = crate::domain::provider::Provider::OpenCode;

    // Step 1: Conflict check
    if ops.check_conflict(materialised_name)? {
        return Err(conflict_failure(materialised_name));
    }

    // Step 2: Pre-register
    let preregistered = ops.preregister(server, materialised_name)?;
    let preregistration = preregistered.report.clone();

    // Extract client_id and secret
    let client_id = preregistered.client_id.ok_or_else(|| {
        Failure::blocked(
            "figma_oauth.no_client_id",
            "internal OAuth flow requires a client_id from pre-registration",
        )
    })?;
    let client_secret = if preregistered.auth_mode == AuthMode::None {
        None
    } else {
        Some(preregistered.secret.map(|(_, s)| s).ok_or_else(|| {
            Failure::blocked(
                "figma_oauth.no_client_secret",
                "internal OAuth flow requires a client secret from pre-registration",
            )
        })?)
    };

    // Step 3: Discover endpoints
    let server_url = server.url.as_deref().ok_or_else(|| {
        Failure::blocked(
            "figma_oauth.no_server_url",
            "internal OAuth flow requires a server URL for endpoint discovery",
        )
    })?;
    let endpoints = ops.discover(server_url)?;

    // Step 4: PKCE + state
    let (verifier, challenge) = oauth::pkce_pair();
    let state = oauth::state();

    // Step 5: Bind listener
    let listener = ops.bind(&state.0)?;

    // Step 6: Print URL
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
    ops.output_url(&auth_url);

    // Step 7: Wait for callback
    let code = ops.wait_code(listener)?;

    // Step 8: Exchange code
    let tokens = ops.exchange(
        &endpoints.token_endpoint,
        &code.0,
        &verifier.0,
        &client_id,
        client_secret.as_deref(),
        preregistered.auth_mode,
        endpoints.resource.as_deref(),
    )?;

    // Step 9: Persist
    let entry = Entry {
        server_url: server_url.to_owned(),
        client_info: ClientInfo {
            client_id,
            client_secret,
            client_secret_expires_at: None,
        },
        tokens,
    };
    ops.write(materialised_name, &entry)?;

    // Step 10: Verify
    if !ops.verify(materialised_name)? {
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
fn conflict_failure(materialised_name: &str) -> Failure {
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
