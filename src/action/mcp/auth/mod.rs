//! `ivar mcp auth <server-name>` — authenticate one of the hall's declared MCP
//! servers under the session's provider, one provider, or (`--all-providers`)
//! every provider the hall lists.
//!
//! # Three steps, one provider at a time
//!
//! 1. **Resolve.** Find `server-name` in `ivar.json`'s `mcp` array, and the
//!    provider(s): `--provider`, the hall's default, or — with
//!    `--all-providers` — every entry in `providers.available`
//!    (`R-PROVIDER`). The two flags are mutually exclusive at the CLI layer
//!    (`clap`'s `conflicts_with`), so this module never has to referee a
//!    combination that makes no sense. An unknown server is a [`Failure`]
//!    that lists every name that *is* declared. That failure must never read
//!    the same as "the provider's own CLI is missing" — the other way this
//!    command can refuse (`R-ERRORS`); the two come from entirely different
//!    code paths (`resolve_server` here versus [`proc::inherit`]'s own
//!    [`Failure`] conversion) rather than one branch trying to tell them
//!    apart after the fact.
//! 2. **Pre-register, only when needed.** OpenCode's own dynamic client
//!    registration is rejected by Figma's MCP server before a browser ever
//!    opens — [`crate::infra::figma`] exists for exactly that gap. When the
//!    provider is OpenCode, the server's `url` host needs a pre-registration
//!    ([`figma::needs_preregistration`]), and the manifest's entry for it
//!    does not already carry an `oauth` client, this registers one with
//!    Figma, writes `oauth.client_id` and `oauth.client_secret_env` back into
//!    `ivar.json`, persists the client secret into hall-local, gitignored
//!    `.ivar/secrets/mcp.env`, and re-materialises `opencode.json` so it picks
//!    the client up. A manifest entry that already carries `oauth`
//!    skips re-registration outright — never re-registered, never invalidated
//!    (`R-IDEMPOTENT`) — and resolves the secret from caller environment or
//!    `.ivar/secrets/mcp.env`. Every other combination (a different provider, a
//!    server with no `url`, a host Figma never gated) is
//!    [`Preregistration::NotNeeded`].
//! 3. **Dispatch.** For omp always, and for OpenCode on Figma hosts, Ivar
//!    performs the OAuth authorization-code flow itself
//!    ([`dispatch::internal_flow`]): endpoint discovery, URL print, callback
//!    listener, code exchange, and a credential-store write that replaces any
//!    existing entry. For Claude Code and OpenCode on non-Figma servers,
//!    the harness's own login command — `claude mcp login <name>` or
//!    `opencode mcp auth <name>` — runs through [`proc::inherit`].
//!    `inherit` is not optional for the provider-owned path: the command
//!    prints a URL and waits on a browser the user is watching, and
//!    capturing its output would freeze that prompt instead of showing it.
//!
//! # `--all-providers` runs sequentially, never concurrently (`R-ALL-SEQUENTIAL`)
//!
//! Both harnesses' login commands take over the whole terminal and wait on a
//! browser. Two running at once would fight over the same terminal and the
//! same default browser tab — there is no output to interleave, only a
//! collision. [`run_provider`] is called once per provider, in a plain
//! sequential loop over `providers.available`, and the next provider does not
//! start until [`proc::inherit`] returns for the one before it.
//!
//! # Partial failure is not success (`R-ALL-PARTIAL`)
//!
//! `--all-providers` attempts *every* provider even after one fails —
//! stopping early would hide whether the remaining providers work, and the
//! whole point of asking for "every provider" is to learn about all of them
//! in one run. Each attempt becomes one [`ProviderRun`] in
//! [`AuthOutcome::runs`], carrying its own [`Preregistration`] and command
//! rather than one collapsed result. A run with any `authenticated: false`
//! entry reports itself as unclean — one [`crate::error::Warning`] per failed
//! provider — so [`crate::error::Report::is_clean`] is `false` and the binary
//! exits `1`, never `0`. One provider succeeding must never look like the
//! whole run succeeding.
//!
//! The single-provider path (no `--all-providers`) keeps its original,
//! stricter contract: a dispatch failure there is still a hard [`Failure`]
//! (exit `2`), because a single explicit request that could not be completed
//! is "broke mid-flight", not "seven of eight went fine". [`try_run_provider`]
//! is that path; [`run_provider`] is `--all-providers`'s never-propagating
//! twin. Both share [`attempt`], which does the actual steps 2 and 3 exactly
//! once — the only difference between the two callers is what they do with
//! its result.
//!
//! # A registration is not an authentication (`R-HONEST`)
//!
//! [`ProviderRun`] reports steps 2 and 3 as two separate facts, and
//! [`WriteHuman`] never lets the first read like the second.
//!
//! Reaching `authenticated: true` used to mean only "the harness's exit
//! status was 0", and live execution found that is not enough on OpenCode:
//! `opencode mcp auth` exits `0` unconditionally — measured against a server
//! name that does not exist, and measured while it printed `Authentication
//! failed` to the terminal — while `claude mcp login` does exit non-zero on
//! failure (also measured). [`verify_authenticated`] is the fix: after a
//! zero exit, it checks the thing itself — [`providers::verify_authenticated`] for
//! OpenCode, a no-op for Claude Code, where the exit status already checked
//! by [`attempt`] is reliable. `authenticated: true` now means that check
//! passed, not merely that the child returned.
//!
//! # Rejected: skip a provider that is already authenticated
//!
//! Considered and turned down — see the Analysis's "Authenticating every
//! provider" section. `ivar` can read OpenCode's `tokens`, but Claude Code's
//! credential store is opaque to it, so the check would exist for one
//! provider and not the other; and `claude mcp login` was measured
//! (2026-08-26) to restart its flow regardless of existing credentials, so
//! the redundant round-trip cannot be avoided for that provider anyway. An
//! asymmetric skip would buy inconsistency, not savings.
//!
//! # Rejected: caching whether a server needs pre-registration
//!
//! [`figma::needs_preregistration`] is re-checked on every run rather than
//! remembered anywhere. The allowlist is Figma's, not `ivar`'s, and it can
//! change without notice (`R-CONTAINED`) — nothing here may assume today's
//! answer stays true tomorrow.
//!
//! # Rejected: OpenCode's own `mcp-auth.json` as the idempotency check
//!
//! An earlier version checked OpenCode's own credential store — did
//! it already have a usable client — before
//! registering. Measured on 2026-08-26: `opencode mcp auth` never reads that
//! store; it resolves its OAuth client from `opencode.json` only. A check
//! against a file the command never consults cannot protect anything, so
//! [`preregister_if_needed`] now checks the one place that actually reaches
//! the command: the manifest's own `McpServerDef.oauth`. See
//! `plans/ivar-mcp-auth/analysis.md`.
//!
//! # The secret storage and handoff (`R-SECRET-HANDOFF`)
//!
//! Figma's registration returns a `client_secret`, and Figma's token endpoint
//! requires it despite echoing `token_endpoint_auth_method: "none"`
//! (measured 2026-08-26). That value is stored in local, gitignored
//! `.ivar/secrets/mcp.env` with owner-only Unix permissions (`0600`) and
//! passed in-memory to the child process environment via [`Attempt`]'s call
//! into [`auth_command`]. It never reaches `ivar.json` (which stores only
//! `oauth.client_secret_env`, the variable's *name*), never `opencode.json`
//! (which stores the `{env:NAME}` reference `harness::config::mcp` renders),
//! and never [`AuthOutcome`] or any type reachable from it, since that struct is
//! `Serialize` and a `--json` run would print it verbatim. [`secret_env_var`]
//! is the one function that names the variable — [`preregister_if_needed`]
//! stores its output in the manifest and persists the value under that key in
//! `.ivar/secrets/mcp.env`.
//!
//! # The secret must reach the very run that mints it (defect fix)
//!
//! When pre-registration mints a new client secret, [`preregister_if_needed`]
//! returns the secret alongside [`Preregistration::Registered`] (in
//! [`Preregistered`], which is deliberately not `Serialize`), and [`attempt`]
//! passes it straight into [`auth_command`]'s [`proc::Command::env`] for that
//! child — never to Claude, never to `ivar.json`, and never to `--json` output.
//! disk, never for a second run. On the [`Preregistration::Skipped`] path —
//! a manifest that already carries `oauth` — `ivar` never held this run's
//! secret in the first place, so [`ensure_secret_env_set`] fails early, naming the
//! variable, rather than let a missing export surface later as the same
//! confusing `client_secret_basic` error.

use crate::action::Ctx;
use crate::domain::provider::Provider;
use crate::error::{Outcome, Report};
use crate::providers;

use super::super::{discover_hall, read_manifest};
use super::{resolve_provider, resolve_server};

mod dispatch;
mod flow;
mod outcome;
mod preregister;
mod requirement;

use dispatch::{run_provider, try_run_provider};
pub use outcome::{AuthMethod, AuthOutcome, Preregistration, ProviderRun};
use outcome::{all_providers_report, dropped_grants};
use requirement::record_auth_requirement;

/// What `ivar mcp auth` needs.
#[derive(Debug, Clone)]
pub struct AuthInput {
    /// The server's name, as declared in `ivar.json`'s `mcp` array.
    pub server: String,
    /// The provider to authenticate against. `None` uses the hall's default.
    /// Mutually exclusive with `all_providers` — the CLI layer enforces this
    /// with `clap`'s `conflicts_with` before either ever reaches here.
    pub provider: Option<String>,
    /// Authenticate every provider in `providers.available`, sequentially
    /// (`R-ALL-SEQUENTIAL`), reporting each one's result even after an
    /// earlier one failed (`R-ALL-PARTIAL`).
    pub all_providers: bool,
}

pub(super) fn provider_order(available: &[Provider]) -> Vec<Provider> {
    let mut order = Vec::with_capacity(available.len());
    if available.contains(&Provider::ClaudeCode) {
        order.push(Provider::ClaudeCode);
    }
    for &p in available {
        if p != Provider::ClaudeCode {
            order.push(p);
        }
    }
    order
}

/// Authenticate `input.server` under `input.provider`, the hall's default, or
/// — with `input.all_providers` — every provider the hall lists.
///
/// See the module doc comment for the steps, the sequential/partial-failure
/// contract of `--all-providers`, and what each failure mode means.
pub fn auth(ctx: &Ctx, input: &AuthInput) -> Outcome<AuthOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    let server = resolve_server(&manifest, &input.server)?;
    // The materialised name is what actually runs — the provider's login
    // command and its own credential store — while `AuthOutcome.server`
    // below keeps the canonical name the operator typed.
    let materialised_name = server.materialised_name(manifest.name());

    if input.all_providers {
        let owners: Vec<Provider> = manifest.providers().config_owners().collect();
        let ordered = provider_order(&owners);
        let server_url = server.url.as_deref().unwrap_or_default();
        let runs: Vec<ProviderRun> = ordered
            .iter()
            .map(|&provider| run_provider(&layout, &manifest, server, &materialised_name, provider))
            .collect();
        let ran = runs.len();

        let recorded = record_auth_requirement(
            &layout,
            &runs,
            server,
            &materialised_name,
            providers::credential_state,
        );
        let mut report = all_providers_report(&server.name, runs);
        report.warnings.extend(recorded);

        if ran >= 2 {
            let mut after = std::collections::BTreeMap::new();
            for &p in &owners {
                match p {
                    Provider::ClaudeCode | Provider::OpenCode | Provider::T3 => {
                        if let Some(Ok(map)) = crate::providers::live_states(p, layout.root())
                            && let Some(&st) = map.get(&materialised_name)
                        {
                            after.insert(p, st);
                        }
                    }
                    Provider::Omp => {
                        let st =
                            crate::providers::credential_state(p, &materialised_name, server_url);
                        after.insert(p, st);
                    }
                }
            }
            let dropped = dropped_grants(&server.name, &report.value.runs, &after);
            for warning in dropped {
                report.warn(warning);
            }
        }

        return Ok(report);
    }

    let provider = resolve_provider(&manifest, input.provider.as_deref())?;
    let run = try_run_provider(&layout, &manifest, server, &materialised_name, provider)?;
    let runs = vec![run];
    let recorded = record_auth_requirement(
        &layout,
        &runs,
        server,
        &materialised_name,
        providers::credential_state,
    );
    Ok(Report::with_warnings(
        AuthOutcome {
            server: server.name.clone(),
            runs,
        },
        recorded.into_iter().collect(),
    ))
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/mcp/auth/support.rs"]
mod support;

#[cfg(test)]
#[path = "../../../../tests/unit/action/mcp/auth.rs"]
mod tests;
