//! Steps 2 and 3 of `ivar mcp auth`, dispatched: run pre-registration and the
//! harness's own login command for one provider, either propagating a
//! failure (the single-provider path) or folding it into a [`ProviderRun`]
//! (`--all-providers`, `R-ALL-PARTIAL`). See `auth/mod.rs`'s module doc
//! comment for the three-step narrative this completes.
//!
//! Additionally, this module owns the internal-flow path for OpenCode + Figma:
//! [`attempt`] dispatches to [`figma_oauth::run_internal_flow`] instead of
//! `proc::inherit` when the provider is OpenCode and the server host is on
//! Figma's pre-registration allowlist.

use crate::domain::mcp::McpServerDef;
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::harness::{Harness, opencode_auth};
use crate::infra::figma;
use crate::infra::proc;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::preregister::{Preregistered, host_of, preregister_if_needed};
use super::{AuthMethod, Preregistration, ProviderRun};

use super::figma_oauth;

/// Steps 2 and 3 for one provider, run exactly once. Both
/// [`try_run_provider`] (the single-provider path, which propagates a
/// failure immediately) and [`run_provider`] (`--all-providers`, which never
/// propagate) are thin wrappers over this - the only difference between the
/// two callers is what they do with [`Attempt::outcome`].
pub(super) struct Attempt {
    preregistration: Preregistration,
    command: String,
    auth_method: AuthMethod,
    outcome: Result<(), Failure>,
}

fn provider_is_internal_flow(provider: Provider, server: &McpServerDef) -> bool {
    provider == Provider::OpenCode && server.url.as_deref().is_some_and(|u| host_of(u).is_some_and(figma::needs_preregistration))
}

fn attempt(
    layout: &Layout,
    manifest: &Manifest,
    server: &McpServerDef,
    materialised_name: &str,
    provider: Provider,
) -> Attempt {
    // For OpenCode + Figma host, dispatch to the internal OAuth flow.
    if provider_is_internal_flow(provider, server)
        && server
            .url
            .as_deref()
            .is_some_and(|u| host_of(u).is_some_and(figma::needs_preregistration))
    {
        return attempt_internal_flow(layout, manifest, server, materialised_name);
    }

    // All other paths: use the original provider-owned path via
    // `proc::inherit`.
    let Preregistered {
        report: preregistration,
        client_id: _,
        secret,
    } = match preregister_if_needed(layout, manifest, provider, server, materialised_name) {
        Ok(preregistered) => preregistered,
        Err(failure) => {
            return Attempt {
                preregistration: Preregistration::NotNeeded,
                command: String::new(),
                auth_method: AuthMethod::ProviderCommand,
                outcome: Err(failure),
            };
        }
    };

    let harness = match Harness::for_provider(provider) {
        Ok(harness) => harness,
        Err(failure) => {
            return Attempt {
                preregistration,
                command: String::new(),
                auth_method: AuthMethod::ProviderCommand,
                outcome: Err(failure),
            };
        }
    };

    let command = auth_command(harness, materialised_name, secret.as_ref());
    let display = command.display();
    let outcome = match proc::inherit(&command) {
        Ok(Some(0)) => verify_authenticated(harness, materialised_name),
        Ok(code) => Err(login_failed(&display, code)),
        Err(spawn_error) => Err(spawn_error.into()),
    };

    Attempt {
        preregistration,
        command: display,
        auth_method: AuthMethod::ProviderCommand,
        outcome,
    }
}

fn attempt_internal_flow(
    layout: &Layout,
    manifest: &Manifest,
    server: &McpServerDef,
    materialised_name: &str,
) -> Attempt {
    match figma_oauth::run_internal_flow_inner(layout, manifest, server, materialised_name) {
        Ok(run) => Attempt {
            preregistration: run.preregistration.clone(),
            command: run.command.clone(),
            auth_method: run.auth_method.clone(),
            outcome: Ok(()),
        },
        Err(failure) => Attempt {
            preregistration: Preregistration::NotNeeded,
            command: String::new(),
            auth_method: AuthMethod::InternalOAuthFlow,
            outcome: Err(failure),
        },
    }
}

/// The single-provider path: propagate [`Attempt::outcome`] immediately. A
/// dispatch failure here is still a hard [`Failure`] (exit `2`) - a single
/// explicit request that could not be completed is "broke mid-flight", the
/// same severity `action/sync/setup.rs` gives an inherited process's
/// non-zero exit.
pub(super) fn try_run_provider(
    layout: &Layout,
    manifest: &Manifest,
    server: &McpServerDef,
    materialised_name: &str,
    provider: Provider,
) -> Result<ProviderRun, Failure> {
    let Attempt {
        preregistration,
        command,
        auth_method,
        outcome,
    } = attempt(layout, manifest, server, materialised_name, provider);
    outcome?;
    Ok(ProviderRun {
        provider,
        preregistration,
        command,
        auth_method,
        authenticated: true,
        error: None,
    })
}

/// `--all-providers`'s path: never propagate. Every attempt becomes a
/// [`ProviderRun`] - success or failure - so the loop in [`auth`] keeps going
/// to the next provider regardless (`R-ALL-PARTIAL`).
pub(super) fn run_provider(
    layout: &Layout,
    manifest: &Manifest,
    server: &McpServerDef,
    materialised_name: &str,
    provider: Provider,
) -> ProviderRun {
    let Attempt {
        preregistration,
        command,
        auth_method,
        outcome,
    } = attempt(layout, manifest, server, materialised_name, provider);
    match outcome {
        Ok(()) => ProviderRun {
            provider,
            preregistration,
            command,
            auth_method,
            authenticated: true,
            error: None,
        },
        Err(failure) => ProviderRun {
            provider,
            preregistration,
            command,
            auth_method,
            authenticated: false,
            error: Some(failure.what),
        },
    }
}

/// The harness's own login command for `materialised_name` - the whole of
/// step 3. The materialised name, not the canonical one: it is what the
/// provider's own config keys this server by, so it is what the login
/// command and OpenCode's `mcp-auth.json` must agree on.
///
/// `secret`, when `Some((var, value))`, is set on the child's own
/// environment (defect fix, `R-SECRET-HANDOFF`): either a fresh registration or
/// an existing one resolved from the caller environment or local store.
fn auth_command(
    harness: Harness,
    materialised_name: &str,
    secret: Option<&(String, String)>,
) -> proc::Command {
    let args: [&str; 2] = match harness {
        Harness::ClaudeCode => ["mcp", "login"],
        Harness::OpenCode => ["mcp", "auth"],
    };
    let command = proc::Command::new(harness.binary())
        .args(args)
        .arg(materialised_name);
    match secret {
        Some((var, value)) => command.env(var.clone(), value.clone()),
        None => command,
    }
}

/// Step 3's exit-0 answer is not enough to believe on every provider
/// (defect fix, `R-HONEST` - see the module doc comment). OpenCode's own
/// `opencode mcp auth` exits `0` unconditionally, so this checks the thing
/// itself: whether a token exchange actually landed in OpenCode's own
/// store. Claude Code's exit status, already checked by [`attempt`] before
/// this runs, is reliable - this is a no-op for it.
///
/// `materialised_name` - never the canonical one - because OpenCode keys
/// `mcp-auth.json` by whatever name [`auth_command`] handed its login
/// command; a bare canonical lookup here would report every successful
/// OpenCode auth as `mcp.auth_not_verified`.
fn verify_authenticated(harness: Harness, materialised_name: &str) -> Result<(), Failure> {
    match harness {
        Harness::ClaudeCode => Ok(()),
        Harness::OpenCode => {
            if opencode_auth::has_tokens(materialised_name)? {
                return Ok(());
            }
            Err(Failure::failed(
                "mcp.auth_not_verified",
                format!(
                    "`opencode mcp auth {materialised_name}` exited 0, but no tokens for \
                     `{materialised_name}` were found in OpenCode's own credential store"
                ),
            )
            .expected("a `tokens` entry for this server in OpenCode's mcp-auth.json")
            .actual(
                "no tokens present - `opencode mcp auth` exits 0 even when it prints \
                 `Authentication failed` (measured 2026-08-26)",
            )
            .fix(FixAction::safe(
                "mcp.retry_auth",
                "Read the command's output above, then run `ivar mcp auth` again.",
            )))
        }
    }
}

/// The harness's own login command exited non-zero, or died to a signal.
fn login_failed(display: &str, code: Option<i32>) -> Failure {
    let ended = match code {
        Some(code) => format!("exited {code}"),
        None => "was killed by a signal".to_owned(),
    };
    Failure::failed("mcp.auth_failed", format!("`{display}` {ended}"))
        .expected("the harness's login command to exit 0")
        .actual(ended)
        .fix(FixAction::safe(
            "mcp.retry_auth",
            "Read the command's output above, then run `ivar mcp auth` again.",
        ))
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/mcp/auth/dispatch.rs"]
mod tests;
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::mcp::McpServerDef;
    use crate::domain::provider::Provider;

    #[test]
    fn opencode_figma_selects_internal_oauth() {
        let server = McpServerDef::new("figma", "sse").url("https://mcp.figma.com/mcp");
        assert!(provider_is_internal_flow(Provider::OpenCode, &server));
    }
    #[test]
    fn claude_figma_selects_provider_command() {
        let server = McpServerDef::new("figma", "sse").url("https://mcp.figma.com/mcp");
        assert!(!provider_is_internal_flow(Provider::ClaudeCode, &server));
    }
    #[test]
    fn opencode_non_figma_selects_provider_command() {
        let server = McpServerDef::new("linear", "sse").url("https://mcp.linear.app/mcp");
        assert!(!provider_is_internal_flow(Provider::OpenCode, &server));
    }
}
