//! Steps 2 and 3 of `ivar mcp auth`, dispatched: run pre-registration and the
//! harness's own login command for one provider, either propagating a
//! failure (the single-provider path) or folding it into a [`ProviderRun`]
//! (`--all-providers`, `R-ALL-PARTIAL`). See `auth/mod.rs`'s module doc
//! comment for the three-step narrative this completes.
//!
//! Additionally, this module owns the internal-flow path for OpenCode + Figma:
//! [`attempt`] dispatches to [`flow::run_internal_flow`] instead of
//! `proc::inherit` when the provider is OpenCode and the server host is on
//! Figma's pre-registration allowlist.

use crate::domain::mcp::McpServerDef;
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::infra::figma;
use crate::infra::proc;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::preregister::{Preregistered, host_of, preregister_if_needed};
use super::{AuthMethod, Preregistration, ProviderRun};

use super::flow;

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
    matches!(provider, Provider::OpenCode | Provider::Omp)
        && server
            .url
            .as_deref()
            .is_some_and(|u| host_of(u).is_some_and(figma::needs_preregistration))
}

fn attempt(
    layout: &Layout,
    manifest: &Manifest,
    server: &McpServerDef,
    materialised_name: &str,
    provider: Provider,
) -> Attempt {
    // For OpenCode / Omp + Figma host, dispatch to the internal OAuth flow.
    if provider_is_internal_flow(provider, server) {
        return attempt_internal_flow(layout, manifest, server, materialised_name, provider);
    }

    // All other paths: use the original provider-owned path via
    // `proc::inherit`.
    let Preregistered {
        report: preregistration,
        client_id: _,
        secret,
        auth_mode: _,
    } = match preregister_if_needed(layout, manifest, provider, server, materialised_name, None) {
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

    // A provider with no MCP login command cannot be driven through this
    // path at all. `omp` is the case today: it has no `mcp` subcommand, and
    // its credential surface is `omp auth-broker` (measured against
    // omp/18.1.8). Refuse before spawning rather than build a command whose
    // binary would reject the arguments.
    let subcommand = match crate::providers::login_subcommand(provider) {
        Some(subcommand) => subcommand,
        None => {
            return Attempt {
                preregistration,
                command: String::new(),
                auth_method: AuthMethod::ProviderCommand,
                outcome: Err(Failure::blocked(
                    "harness.unsupported",
                    format!("`{provider}` has no MCP login command"),
                )
                .expected("a provider whose CLI can run an MCP login")
                .actual(format!(
                    "`{}` exposes no `mcp` subcommand",
                    crate::providers::launch_contract(provider).binary
                ))),
            };
        }
    };

    let command = auth_command(provider, subcommand, materialised_name, secret.as_ref());
    let display = command.display();
    let outcome = match proc::inherit(&command) {
        Ok(Some(0)) => crate::providers::verify_authenticated(
            provider,
            materialised_name,
            server.url.as_deref(),
        ),
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
    provider: Provider,
) -> Attempt {
    match flow::run_internal_flow_inner(layout, manifest, server, materialised_name, provider) {
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
    provider: Provider,
    subcommand: [&str; 2],
    materialised_name: &str,
    secret: Option<&(String, String)>,
) -> proc::Command {
    let binary = crate::providers::launch_contract(provider).binary;
    let command = proc::Command::new(binary)
        .args(subcommand)
        .arg(materialised_name);
    match secret {
        Some((var, value)) => command.env(var.clone(), value.clone()),
        None => command,
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
