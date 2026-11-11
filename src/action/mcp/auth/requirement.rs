//! # Recording that a server requires auth
//!
//! A server that registers or authenticates itself never gets an `oauth` block,
//! so nothing in `ivar.json` would say it needs a token and `ivar mcp status`
//! would call it `not-required`. Once every run is done, [`super::auth`] checks each
//! successful run's provider credential store ([`has_credential_evidence`]); if
//! one holds a present or refreshable token, it re-reads the manifest and
//! writes `auth = "oauth"` once ([`mark_oauth_required`]). A zero exit alone is
//! never enough — `claude mcp login` exits `0` for a server that needs no
//! auth — and the write is skipped when `oauth` or `auth` is already on file.

use crate::domain::mcp::{CredentialState, McpAuth, McpServerDef};
use crate::domain::provider::Provider;
use crate::error::Warning;
use crate::store::manifest::Manifest;

use super::super::super::read_manifest;
use super::ProviderRun;

/// Record in `ivar.json` that `server` requires OAuth, once a run has left a
/// credential behind. Returns the warning to surface when the manifest could
/// not be updated — the login itself already succeeded, so this never fails
/// the command.
pub(super) fn record_auth_requirement(
    layout: &crate::store::layout::Layout,
    runs: &[ProviderRun],
    server: &McpServerDef,
    materialised_name: &str,
    state_of: impl Fn(Provider, &str, &str) -> CredentialState,
) -> Option<Warning> {
    // A server already known to require auth has nothing left to record;
    // skipping here also spares the credential-store reads.
    if server.oauth.is_some() || server.auth.is_some() {
        return None;
    }
    let url = server.url.as_deref()?;
    if !has_credential_evidence(runs, materialised_name, url, state_of) {
        return None;
    }
    mark_oauth_required(layout, &server.name)
        .err()
        .map(|failure| {
            Warning::new(
                "mcp.auth_not_recorded",
                server.name.clone(),
                format!(
                    "authenticated, but could not record that `{}` requires auth in ivar.json: {}",
                    server.name, failure.what
                ),
            )
        })
}

/// Whether some successful run left a credential in its provider's store.
///
/// A zero exit is not evidence: `claude mcp login` also exits `0` for a server
/// that needs no auth, and
/// [`Preregistration::NoAuthRequired`](super::Preregistration::NoAuthRequired)
/// reports `authenticated` with nothing stored. Only a token that is present (or
/// refreshable) shows the server demanded one.
fn has_credential_evidence(
    runs: &[ProviderRun],
    materialised_name: &str,
    server_url: &str,
    state_of: impl Fn(Provider, &str, &str) -> CredentialState,
) -> bool {
    runs.iter().filter(|run| run.authenticated).any(|run| {
        matches!(
            state_of(run.provider, materialised_name, server_url),
            CredentialState::Authenticated | CredentialState::Expired
        )
    })
}

/// Set `auth = oauth` on the manifest's `server_name`, re-reading the manifest
/// first so a client the pre-registration step just wrote is never clobbered.
/// Returns whether the manifest was rewritten: a server that already carries
/// `oauth` or `auth` is left untouched (`R-IDEMPOTENT`).
fn mark_oauth_required(
    layout: &crate::store::layout::Layout,
    server_name: &str,
) -> Result<bool, crate::error::Failure> {
    let manifest = read_manifest(layout)?;
    let Some(current) = manifest
        .mcp_servers()
        .iter()
        .find(|s| s.name == server_name)
    else {
        return Ok(false);
    };
    if current.oauth.is_some() || current.auth.is_some() {
        return Ok(false);
    }
    let servers: Vec<McpServerDef> = manifest
        .mcp_servers()
        .iter()
        .map(|s| {
            if s.name == server_name {
                s.clone().auth(McpAuth::Oauth)
            } else {
                s.clone()
            }
        })
        .collect();
    let updated = manifest.with_mcp_servers(servers)?;
    Manifest::write(layout, &updated)?;
    Ok(true)
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/mcp/auth/requirement.rs"]
mod tests;
