//! Step 2 of `ivar mcp auth`: pre-register an OAuth client with Figma when
//! (and only when) the provider is OpenCode and the manifest's entry has no
//! usable registration yet. See `auth/mod.rs`'s module doc comment for how
//! this fits into the three-step narrative and the secret-handoff contract
//! (`R-SECRET-HANDOFF`).

use std::io::{self, Write};

use crate::domain::mcp::{McpOauth, McpServerDef};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::harness::config;
use crate::infra::figma;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::Preregistration;

/// What step 2 did, plus — only for a fresh registration — the secret the
/// dispatched child needs in its own environment (defect fix, see the module
/// doc comment's "The secret must reach the very run that mints it" section).
///
/// Deliberately not `Serialize`: unlike [`Preregistration`], this type must
/// never become reachable from [`AuthOutcome`] or a `--json` run would print
/// the secret verbatim.
#[derive(Debug)]
pub(super) struct Preregistered {
    /// What [`ProviderRun`] reports for step 2.
    pub(super) report: Preregistration,
    /// `(env var name, secret value)`, present only when this call just
    /// registered a brand-new client. `None` for `NotNeeded` and `Skipped` —
    /// in both cases `ivar` never held a secret to hand off.
    pub(super) fresh_secret: Option<(String, String)>,
}

impl Preregistered {
    fn not_needed() -> Self {
        Self {
            report: Preregistration::NotNeeded,
            fresh_secret: None,
        }
    }

    fn skipped() -> Self {
        Self {
            report: Preregistration::Skipped,
            fresh_secret: None,
        }
    }
}

/// Step 2: pre-register a client with Figma when, and only when, every
/// condition in the plan holds. Every other combination is
/// [`Preregistration::NotNeeded`] — including a server with no `url` at all,
/// which cannot need a host-based workaround.
///
/// A successful registration writes back to `ivar.json` and re-materialises
/// `opencode.json` before returning — see the module doc comment's "The
/// secret handoff" section for why the secret itself never joins any of
/// that.
pub(super) fn preregister_if_needed(
    layout: &Layout,
    manifest: &Manifest,
    provider: Provider,
    server: &McpServerDef,
) -> Result<Preregistered, Failure> {
    if provider != Provider::OpenCode {
        return Ok(Preregistered::not_needed());
    }
    let Some(url) = server.url.as_deref() else {
        return Ok(Preregistered::not_needed());
    };
    let Some(host) = host_of(url) else {
        return Ok(Preregistered::not_needed());
    };
    if !figma::needs_preregistration(host) {
        return Ok(Preregistered::not_needed());
    }

    // A usable client registration already on the manifest: skip outright,
    // never re-register (`R-IDEMPOTENT`) — a second run must leave a working
    // registration alone. This checks `ivar.json`, not OpenCode's own
    // `mcp-auth.json`: `opencode mcp auth` never reads that store (measured
    // 2026-08-26), so a check against it could never protect anything — see
    // the module doc comment's "Rejected" section.
    if let Some(oauth) = &server.oauth {
        // `ivar` never held this run's secret — the manifest only ever
        // stored the variable's *name*. Fail early, naming it, rather than
        // dispatch into OpenCode's confusing `client_secret_basic
        // authentication requires a client_secret` (`R-ERRORS`).
        ensure_secret_env_set(&oauth.client_secret_env, &server.name)?;
        return Ok(Preregistered::skipped());
    }

    let registered = figma::register_client(config::OAUTH_REDIRECT_URI)?;
    let client_secret = registered.client_secret.ok_or_else(|| {
        Failure::failed(
            "mcp.figma_no_client_secret",
            format!(
                "Figma's registration for `{}` returned no client_secret",
                server.name
            ),
        )
        .expected(
            "a client_secret in the registration response — Figma's token endpoint requires \
             one despite the registration echoing `token_endpoint_auth_method: \"none\"` \
             (measured 2026-08-26)",
        )
        .actual("client_secret absent")
    })?;

    let secret_env = secret_env_var(&server.name);
    let updated_servers: Vec<McpServerDef> = manifest
        .mcp_servers()
        .iter()
        .map(|existing| {
            if existing.name == server.name {
                existing.clone().oauth(McpOauth::new(
                    registered.client_id.clone(),
                    secret_env.clone(),
                ))
            } else {
                existing.clone()
            }
        })
        .collect();
    let updated_manifest = manifest.with_mcp_servers(updated_servers)?;
    Manifest::write(layout, &updated_manifest)?;

    let mcp_path = layout.mcp_config(&Provider::OpenCode);
    config::materialise_mcp(
        &mcp_path,
        Provider::OpenCode,
        updated_manifest.mcp_servers(),
    )?;

    print_secret_export(&secret_env, &client_secret)?;

    Ok(Preregistered {
        report: Preregistration::Registered {
            client_id: registered.client_id,
        },
        fresh_secret: Some((secret_env, client_secret)),
    })
}

/// Refuse before ever dispatching, naming the variable, when a server whose
/// pre-registration was already `Skipped` has no usable secret in the
/// current environment. `ivar` never holds this run's secret in that case —
/// the operator's own `export` is the only source — so a silent dispatch
/// would just relay OpenCode's confusing `client_secret_basic` error.
fn ensure_secret_env_set(var: &str, server_name: &str) -> Result<(), Failure> {
    if std::env::var_os(var).is_some() {
        return Ok(());
    }

    Err(Failure::blocked(
        "mcp.missing_client_secret_env",
        format!(
            "`{var}` is not set — `{server_name}` already has a registered OAuth client, and \
             its secret must come from the operator's environment"
        ),
    )
    .expected(format!("`{var}` set in the environment"))
    .actual(format!("`{var}` is not set"))
    .fix(FixAction::safe(
        "mcp.export_client_secret",
        format!("export {var}=<the client secret>, then run `ivar mcp auth` again."),
    )))
}

/// The environment variable name a fresh registration's secret is exported
/// under, deterministic from `server_name`.
///
/// Every ASCII letter or digit is uppercased; everything else (`-`, mostly)
/// folds to `_` — `figma-gaio` becomes `IVAR_MCP_FIGMA_GAIO_SECRET`. This is
/// the *only* place that name is built: [`preregister_if_needed`] stores this
/// exact string in `ivar.json`'s `oauth.client_secret_env`, and
/// [`print_secret_export`] prints this exact string in the `export` line —
/// both read this function's output rather than formatting the name
/// themselves a second time, so the two can never drift into two different
/// spellings of the same variable.
fn secret_env_var(server_name: &str) -> String {
    let normalised: String = server_name
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

/// Print the `export` line for a freshly registered client's secret to the
/// operator's terminal — the secret's one and only destination
/// (`R-SECRET-HANDOFF`).
///
/// This writes directly to stderr rather than through [`Ctx`]'s progress
/// sink: that sink is transient (redrawn, then erased) and silenced outright
/// for a `--json` run or a non-tty stderr, and neither behaviour is
/// acceptable for the one chance the operator gets to see this value. It is
/// also never folded into [`AuthOutcome`] or anything reachable from it —
/// that struct is `Serialize`, so a field on it would print the secret
/// verbatim the moment a `--json` run existed. `ivar.json` and every file
/// `ivar` materialises hold only the variable's *name*; this call is the one
/// place its *value* is ever written anywhere.
fn print_secret_export(var: &str, secret: &str) -> Result<(), Failure> {
    writeln!(io::stderr(), "export {var}={secret}").map_err(|source| {
        Failure::failed(
            "mcp.print_secret_export",
            format!("could not print the client secret export line: {source}"),
        )
        .expected("a writable stderr")
        .actual(source.to_string())
        .fix(FixAction::unsafe_(
            "mcp.reset_oauth_registration",
            "The registration already succeeded and is recorded in ivar.json, so a plain rerun \
             of `ivar mcp auth` will see oauth already present and skip re-registering \
             (R-IDEMPOTENT) — it will not print this secret again. Once stderr is writable, \
             remove this server's `oauth` entry from ivar.json's `mcp` array and rerun `ivar \
             mcp auth` to register a fresh client and print its secret.",
        ))
    })
}

/// The host portion of a URL: scheme, userinfo, port, path, query and
/// fragment stripped.
///
/// ponytail: no IPv6-literal handling (`[::1]:443` would split on the wrong
/// `:`) — every host this crate matches against today (`mcp.figma.com`) is a
/// plain DNS name. Add bracket-aware parsing if a future allowlist entry ever
/// needs one, rather than pulling in a URL-parsing dependency for one string.
fn host_of(url: &str) -> Option<&str> {
    let authority = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = authority.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    Some(host.split(':').next().unwrap_or(host))
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/mcp/auth/preregister.rs"]
mod tests;
