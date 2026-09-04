//! Step 2 of `ivar mcp auth`: pre-register an OAuth client when the
//! manifest's entry has no usable registration yet. See `auth/mod.rs`'s
//! module doc comment for how this fits into the three-step narrative and the
//! secret-handoff contract (`R-SECRET-HANDOFF`).

use super::Preregistration;
use crate::domain::mcp::{McpOauth, McpServerDef};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::harness::config;
use crate::infra::figma;
use crate::infra::oauth::AuthMode;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::store::mcp_secrets::McpSecrets;

/// The outcome of step 2.
pub(super) struct Preregistered {
    /// What step 2 did, for [`super::ProviderRun::preregistration`].
    pub(super) report: Preregistration,
    /// The OAuth `client_id`, when step 2 produced or resolved one.
    /// Needed by the internal OAuth flow to build the authorize URL and
    /// the token exchange request.
    pub(super) client_id: Option<String>,
    /// Resolved or freshly minted client secret for child command dispatch.
    pub(super) secret: Option<(String, String)>,
    /// The auth mode to use for token exchange.
    pub(super) auth_mode: AuthMode,
}

impl std::fmt::Debug for Preregistered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preregistered")
            .field("report", &self.report)
            .field("client_id", &self.client_id)
            .field("secret_var", &self.secret.as_ref().map(|(var, _)| var))
            .field("auth_mode", &self.auth_mode)
            .finish()
    }
}

impl Preregistered {
    fn not_needed() -> Self {
        Self {
            report: Preregistration::NotNeeded,
            client_id: None,
            secret: None,
            auth_mode: AuthMode::None,
        }
    }
}

/// Step 2: pre-register a client with Figma when, and only when, every
/// condition in the plan holds. Every other combination is
/// [`Preregistration::NotNeeded`] — including a server with no `url` at all,
/// which cannot need a host-based workaround.
///
/// A successful registration persists the secret to `.ivar/secrets/mcp.env`,
/// writes back to `ivar.json`, and re-materialises `opencode.json` before returning.
const CLIENT_NAME: &str = "ivar";

pub(super) fn preregister_if_needed(
    layout: &Layout,
    manifest: &Manifest,
    provider: Provider,
    server: &McpServerDef,
    materialised_name: &str,
    endpoints: Option<&crate::infra::mcp_oauth::OAuthEndpoints>,
) -> Result<Preregistered, Failure> {
    // A usable client registration already on the manifest: skip outright,
    // never re-register (`R-IDEMPOTENT`) — a second run must leave a working
    // registration alone. Resolve the secret from environment or local store.
    if let Some(oauth) = &server.oauth {
        let (secret, auth_mode) = match &oauth.client_secret_env {
            Some(var) => {
                let val = resolve_secret(layout, var, &server.name)?;
                (Some((var.clone(), val)), AuthMode::ClientSecretPost)
            }
            None => (None, AuthMode::None),
        };
        return Ok(Preregistered {
            report: Preregistration::Skipped,
            client_id: Some(oauth.client_id.clone()),
            secret,
            auth_mode,
        });
    }

    let Some(url) = server.url.as_deref() else {
        return Ok(Preregistered::not_needed());
    };
    let Some(host) = host_of(url) else {
        return Ok(Preregistered::not_needed());
    };

    // Figma refuses a generic registration: its allowlist checks
    // `client_name`, so it gets its own registrar. Every other server is
    // registered against the endpoint its own metadata advertised.
    let registered = if figma::needs_preregistration(host) {
        figma::register_client(crate::infra::http_callback::OAUTH_REDIRECT_URI)?
    } else {
        let Some(endpoint) = endpoints.and_then(|ep| ep.registration_endpoint.as_deref()) else {
            return Ok(Preregistered::not_needed());
        };
        crate::infra::mcp_oauth::register_client(
            endpoint,
            crate::infra::http_callback::OAUTH_REDIRECT_URI,
            CLIENT_NAME,
        )?
    };

    let auth_mode = registered.auth_mode();
    let secret_env = match &registered.client_secret {
        Some(secret) => {
            let var = secret_env_var(materialised_name);
            McpSecrets::set_and_write(layout, &var, secret)?;
            Some((var, secret.clone()))
        }
        None => None,
    };

    let updated_servers: Vec<McpServerDef> = manifest
        .mcp_servers()
        .iter()
        .map(|existing| {
            if existing.name == server.name {
                existing.clone().oauth(oauth_registration(
                    &registered.client_id,
                    secret_env.as_ref().map(|(var, _)| var.as_str()),
                    endpoints,
                ))
            } else {
                existing.clone()
            }
        })
        .collect();
    let updated_manifest = manifest.with_mcp_servers(updated_servers)?;
    Manifest::write(layout, &updated_manifest)?;

    let mcp_path = layout.mcp_config(&provider);
    config::materialise_mcp(
        &mcp_path,
        provider,
        updated_manifest.mcp_servers(),
        updated_manifest.name(),
    )?;

    Ok(Preregistered {
        report: Preregistration::Registered {
            client_id: registered.client_id.clone(),
        },
        client_id: Some(registered.client_id),
        secret: secret_env,
        auth_mode,
    })
}

/// The `McpOauth` a fresh registration writes to `ivar.json`.
///
/// The endpoint metadata comes from discovery, which ran before this call and
/// carries what the *server* published — never a constant. `resource` is
/// optional at the source (RFC 8707 is not universal), so it is written only
/// when the metadata carried one; `token_url` is what makes omp's native
/// refresh work, so its absence means the caller skipped discovery entirely.
fn oauth_registration(
    client_id: &str,
    secret_env: Option<&str>,
    endpoints: Option<&crate::infra::mcp_oauth::OAuthEndpoints>,
) -> McpOauth {
    let mut oauth = match secret_env {
        Some(var) => McpOauth::new(client_id, var),
        None => McpOauth::public(client_id),
    };
    if let Some(ep) = endpoints {
        oauth = oauth.token_url(&ep.token_endpoint);
        if let Some(res) = &ep.resource {
            oauth = oauth.resource(res);
        }
    }
    oauth
}

/// Resolve a registered OAuth client secret from the caller's environment first
/// and then from `.ivar/secrets/mcp.env`. If the caller's environment supplied
/// the value, backfills the local store for existing halls.
fn resolve_secret(layout: &Layout, var: &str, server_name: &str) -> Result<String, Failure> {
    if let Ok(env_val) = std::env::var(var) {
        let _ = McpSecrets::set_and_write(layout, var, &env_val)?;
        return Ok(env_val);
    }

    let secrets = McpSecrets::read(layout)?;
    if let Some(stored_val) = secrets.get(var) {
        return Ok(stored_val.to_owned());
    }

    Err(Failure::blocked(
        "mcp.missing_client_secret_env",
        format!(
            "`{var}` is not set — `{server_name}` already has a registered OAuth client, and \
             its secret must come from `.ivar/secrets/mcp.env` or the operator's environment"
        ),
    )
    .expected(format!(
        "`{var}` set in `.ivar/secrets/mcp.env` or the environment"
    ))
    .actual(format!("`{var}` is not set"))
    .fix(FixAction::safe(
        "mcp.export_client_secret",
        format!("export {var}=<the client secret>, then run `ivar mcp auth` again."),
    )))
}

/// The environment variable name a fresh registration's secret is exported
/// under, deterministic from `materialised_name`.
///
/// Every ASCII letter or digit is uppercased; everything else (`-`, mostly)
/// folds to `_` — `acme-figma` becomes `IVAR_MCP_ACME_FIGMA_SECRET`. This is
/// the *only* place that name is built: [`preregister_if_needed`] stores this
/// exact string in `ivar.json`'s `oauth.client_secret_env` and in `.ivar/secrets/mcp.env`.
fn secret_env_var(materialised_name: &str) -> String {
    let normalised: String = materialised_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("IVAR_MCP_{normalised}_SECRET")
}

/// The host portion of a URL: scheme, userinfo, port, path, query and
/// fragment stripped. `None` when `url` has no authority or cannot be
/// parsed.
///
/// Exposed as `pub(super)` so [`super::dispatch`] can gate on
/// [`figma::needs_preregistration`] without duplicating the parsing.
pub(super) fn host_of(url: &str) -> Option<&str> {
    let authority = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = authority.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    Some(host.split(':').next().unwrap_or(host))
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/mcp/auth/preregister.rs"]
mod tests;
