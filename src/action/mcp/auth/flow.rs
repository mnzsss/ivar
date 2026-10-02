//! Internal OAuth flow, owned entirely by Ivar.
//!
//! Ivar runs the full authorization-code + PKCE flow itself, rather than
//! delegating to a harness's own login command, whenever
//! `provider_is_internal_flow` says so: today that is `opencode` against a
//! host which rejects its own dynamic client registration. Failures raised
//! here are keyed `flow.*` — the flow is generic, and only the Figma
//! exception in `infra::figma` raises `figma.*`.
//!
//! 1. **Endpoint discovery** — \[`mcp_oauth::discover_oauth_endpoints`\].
//! 2. **Pre-registration** — reuse [`preregister_if_needed`] for
//!    client_id / client_secret.
//! 3. **PKCE + state** — [`oauth::pkce_pair`] and [`oauth::state`].
//! 4. **Listener** — bind `127.0.0.1:19876` before printing the URL.
//! 5. **Print URL** — authorization URL for manual browser opening.
//! 6. **Wait for callback** — validate state, receive code.
//! 7. **Code exchange** — [`oauth::exchange_code`].
//! 8. **Persist** — [`providers::install_credentials`], replacing any
//!    existing entry for the server.
//! 9. **Verify** — [`providers::verify_authenticated`].
//!
//! Failure at any step before 8 leaves the credential store unchanged
//! (`R-ATOMIC`). `Ctrl+C` terminates the process; the OS releases the
//! loopback socket; nothing partial is written.
//! # Module boundaries
//!
//! `action` may import `domain`, `infra`, `harness`, and `store`. This
//! module reaches into all four layers for the orchestration steps.

use std::io::{self, Write};
use std::time::Duration;

use crate::domain::mcp::McpServerDef;
use crate::domain::provider::Provider;
use crate::error::Failure;
use crate::providers::Credential;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::preregister::{Preregistered, preregister_if_needed};
use super::{AuthMethod, Preregistration, ProviderRun};

use crate::infra::http_callback::{AuthorizationCode, CallbackServer, OAUTH_REDIRECT_URI};
use crate::infra::mcp_oauth::{self, DiscoveryOutcome, OAuthEndpoints};
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
    fn provider(&self) -> crate::domain::provider::Provider;
    fn preregister(
        &self,
        server: &McpServerDef,
        name: &str,
        endpoints: &OAuthEndpoints,
    ) -> Result<Preregistered, Failure>;
    fn discover(&self, url: &str) -> Result<DiscoveryOutcome, Failure>;
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
    fn write(&self, name: &str, credential: &Credential<'_>) -> Result<(), Failure>;
    fn verify(&self, name: &str) -> Result<bool, Failure>;
}

struct RealFlowOps {
    layout: Layout,
    manifest: Manifest,
    provider: crate::domain::provider::Provider,
}

pub(super) fn rematerialise_mcp(layout: &Layout, provider: Provider) -> Result<(), Failure> {
    let Some(manifest) = Manifest::read(layout)? else {
        return Ok(());
    };
    crate::harness::config::materialise_mcp(
        &layout.mcp_config(&provider),
        provider,
        manifest.mcp_servers(),
        manifest.name(),
    )?;
    Ok(())
}

impl FlowOps for RealFlowOps {
    fn provider(&self) -> crate::domain::provider::Provider {
        self.provider
    }
    fn preregister(
        &self,
        server: &McpServerDef,
        name: &str,
        endpoints: &OAuthEndpoints,
    ) -> Result<Preregistered, Failure> {
        preregister_if_needed(
            &self.layout,
            &self.manifest,
            self.provider,
            server,
            name,
            Some(endpoints),
        )
    }
    fn discover(&self, url: &str) -> Result<DiscoveryOutcome, Failure> {
        mcp_oauth::discover_oauth_endpoints(url)
    }
    fn bind(&self, state: &str) -> Result<CallbackServer, Failure> {
        CallbackServer::bind(state, CALLBACK_TIMEOUT)
    }
    fn output_url(&self, url: &str) {
        if let Err(e) = crate::infra::browser::open(url) {
            let _ = writeln!(io::stderr().lock(), "Could not open a browser: {e}");
        }
        let is_tty = crate::infra::term::is_tty(crate::infra::term::Stream::Stderr);
        let rendered = render_url(url, is_tty);
        let _ = writeln!(
            io::stderr().lock(),
            "Open this URL to authenticate:\n\n  {rendered}\n"
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
    fn write(&self, name: &str, credential: &Credential<'_>) -> Result<(), Failure> {
        crate::providers::install_credentials(self.provider, name, credential)?;
        if self.provider == Provider::Omp {
            rematerialise_mcp(&self.layout, self.provider)?;
        }
        Ok(())
    }
    fn verify(&self, name: &str) -> Result<bool, Failure> {
        Ok(crate::providers::verify_authenticated(self.provider, name).is_ok())
    }
}

pub(super) fn run_internal_flow_inner(
    layout: &Layout,
    manifest: &Manifest,
    server: &McpServerDef,
    materialised_name: &str,
    provider: crate::domain::provider::Provider,
) -> Result<ProviderRun, Failure> {
    let ops = RealFlowOps {
        layout: layout.clone(),
        manifest: manifest.clone(),
        provider,
    };
    run_internal_flow_pipeline(&ops, server, materialised_name)
}

pub(super) fn render_url(url: &str, tty: bool) -> String {
    if tty {
        format!("\x1b]8;;{url}\x1b\\{url}\x1b]8;;\x1b\\")
    } else {
        url.to_owned()
    }
}

/// The `scope` parameter for the authorization request: every scope the
/// server advertised, space-delimited per RFC 6749 §3.3.
///
/// Sending only the first would mint a token narrower than the server
/// offers — `read` without `write` on Linear, `openid` without
/// `mcp:custom-audience` on a Keycloak-backed server, where the missing
/// scope is the audience the MCP endpoint checks. Neither failure appears
/// at authentication time.
///
/// `None` when the server advertised nothing, and when it advertised an
/// empty list — an empty `scope=` is a request, not an absence.
pub(super) fn scope_parameter(endpoints: &OAuthEndpoints) -> Option<String> {
    let scopes = endpoints.scopes_supported.as_ref()?;
    if scopes.is_empty() {
        return None;
    }
    Some(scopes.join(" "))
}

pub(super) fn run_internal_flow_pipeline(
    ops: &dyn FlowOps,
    server: &McpServerDef,
    materialised_name: &str,
) -> Result<ProviderRun, Failure> {
    let provider = ops.provider();

    let server_url = server.url.as_deref().ok_or_else(|| {
        Failure::blocked(
            "flow.no_server_url",
            "internal OAuth flow requires a server URL for endpoint discovery",
        )
    })?;

    // Step 1: Discover endpoints
    let endpoints = match ops.discover(server_url)? {
        DiscoveryOutcome::Endpoints(endpoints) => endpoints,
        DiscoveryOutcome::NoAuthRequired => {
            return Ok(ProviderRun {
                provider,
                preregistration: Preregistration::NoAuthRequired,
                auth_method: AuthMethod::InternalOAuthFlow,
                command: INTERNAL_FLOW_LABEL.to_owned(),
                authenticated: true,
                error: None,
            });
        }
    };

    // Step 2: Pre-register
    let preregistered = ops.preregister(server, materialised_name, &endpoints)?;
    let preregistration = preregistered.report.clone();

    // Extract client_id and secret
    let client_id = preregistered.client_id.ok_or_else(|| {
        Failure::blocked(
            "flow.no_client_id",
            "internal OAuth flow requires a client_id from pre-registration",
        )
    })?;
    let client_secret = if preregistered.auth_mode == AuthMode::None {
        None
    } else {
        Some(preregistered.secret.map(|(_, s)| s).ok_or_else(|| {
            Failure::blocked(
                "flow.no_client_secret",
                "internal OAuth flow requires a client secret from pre-registration",
            )
        })?)
    };
    let (verifier, challenge) = oauth::pkce_pair();
    let state = oauth::state();

    // Step 4: Bind listener
    let listener = ops.bind(&state.0)?;

    // Step 5: Print URL
    let scope = scope_parameter(&endpoints);
    let auth_url = oauth::authorize_url(
        &endpoints.authorization_endpoint,
        &client_id,
        REDIRECT_URI,
        &state,
        &challenge,
        endpoints.resource.as_deref(),
        scope.as_deref(),
    );
    ops.output_url(&auth_url);

    // Step 6: Wait for callback
    let code = ops.wait_code(listener)?;

    // Step 7: Exchange code
    let tokens = ops.exchange(
        &endpoints.token_endpoint,
        &code.0,
        &verifier.0,
        &client_id,
        client_secret.as_deref(),
        preregistered.auth_mode,
        endpoints.resource.as_deref(),
    )?;

    // Step 8: Persist
    let credential = Credential {
        server_url,
        client_id: &client_id,
        client_secret: client_secret.as_deref(),
        tokens: &tokens,
    };
    ops.write(materialised_name, &credential)?;

    // Step 9: Verify
    if !ops.verify(materialised_name)? {
        return Err(Failure::failed(
            "flow.verify_failed",
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

#[cfg(test)]
#[path = "../../../../tests/unit/action/mcp/auth/flow.rs"]
mod tests;
