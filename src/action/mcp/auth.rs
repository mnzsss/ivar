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
//!    `ivar.json`, re-materialises `opencode.json` so it picks the client up,
//!    and prints the client secret's `export` line to the operator's
//!    terminal exactly once (`R-SECRET-HANDOFF`) — see
//!    [`print_secret_export`]. A manifest entry that already carries `oauth`
//!    skips the whole step outright — never re-registered, never invalidated
//!    (`R-IDEMPOTENT`). Every other combination (a different provider, a
//!    server with no `url`, a host Figma never gated) is
//!    [`Preregistration::NotNeeded`].
//! 3. **Dispatch.** Hand off to the harness's own login command — `claude mcp
//!    login <name>` or `opencode mcp auth <name>` — through [`proc::inherit`].
//!    `inherit` is not optional here: the command prints a URL and waits on a
//!    browser the user is watching, and capturing its output would freeze
//!    that prompt instead of showing it.
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
//! zero exit, it checks the thing itself — [`opencode_auth::has_tokens`] for
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
//! An earlier version checked `harness::opencode_auth::client_for` — did
//! OpenCode's own credential store already have a usable client — before
//! registering. Measured on 2026-08-26: `opencode mcp auth` never reads that
//! store; it resolves its OAuth client from `opencode.json` only. A check
//! against a file the command never consults cannot protect anything, so
//! [`preregister_if_needed`] now checks the one place that actually reaches
//! the command: the manifest's own `McpServerDef.oauth`. See
//! `plans/ivar-mcp-auth/analysis.md`.
//!
//! # The secret handoff (`R-SECRET-HANDOFF`)
//!
//! Figma's registration returns a `client_secret`, and Figma's token endpoint
//! requires it despite echoing `token_endpoint_auth_method: "none"`
//! (measured 2026-08-26). That value goes to exactly two places, in memory
//! only, and nowhere else: the operator's terminal, via
//! [`print_secret_export`], and — for the one dispatch that mints it — the
//! child's own environment, via [`Attempt`]'s call into [`auth_command`]. It
//! never reaches `ivar.json` (which stores only `oauth.client_secret_env`,
//! the variable's *name*), never `opencode.json` (which stores the
//! `{env:NAME}` reference `harness::config::mcp` renders), and never
//! [`AuthOutcome`] or any type reachable from it, since that struct is
//! `Serialize` and a `--json` run would print it verbatim. [`secret_env_var`]
//! is the one function that names the variable — [`preregister_if_needed`]
//! stores its output in the manifest and [`print_secret_export`] prints it in
//! the `export` line, so the two can never spell the same variable two
//! different ways.
//!
//! # The secret must reach the very run that mints it (defect fix)
//!
//! Live execution found this: `ivar mcp auth` printed the `export` line and
//! immediately dispatched `opencode mcp auth`, which failed with Figma's
//! `client_secret_basic authentication requires a client_secret` — the
//! operator cannot have exported the variable yet on the run that just
//! created it. [`preregister_if_needed`] now returns the fresh secret
//! alongside [`Preregistration::Registered`] (in [`Preregistered`], which is
//! deliberately not `Serialize`), and [`attempt`] passes it straight into
//! [`auth_command`]'s [`proc::Command::env`] for that one child — never to
//! disk, never for a second run. On the `Skipped` path — a manifest that
//! already carries `oauth` — `ivar` never held this run's secret in the
//! first place, so [`ensure_secret_env_set`] fails early, naming the
//! variable, rather than let a missing export surface later as the same
//! confusing `client_secret_basic` error.

use std::io::{self, Write};

use serde::Serialize;

use crate::action::Ctx;
use crate::domain::mcp::{McpOauth, McpServerDef};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::harness::{Harness, config, opencode_auth};
use crate::infra::{figma, proc};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::{discover_hall, read_manifest};

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

/// Whether step 2 (pre-registration) ran, and what it found.
///
/// Never conflated with authentication — see the module doc comment's
/// `R-HONEST` section. [`WriteHuman`] gives each variant its own sentence so a
/// reader cannot mistake "a client is registered" for "this server is
/// authenticated".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Preregistration {
    /// The provider isn't OpenCode, the server has no `url`, or its host
    /// isn't one Figma's allowlist gates — step 2 never applied.
    NotNeeded,
    /// A usable client registration was already on the manifest; nothing was
    /// registered or touched (`R-IDEMPOTENT`).
    Skipped,
    /// A new client was registered with Figma, written into `ivar.json`
    /// (`oauth.client_id` and `oauth.client_secret_env`), and materialised
    /// into `opencode.json`.
    Registered {
        /// The `client_id` Figma issued. Not a secret by itself, but never
        /// reported alongside the client secret either — that never leaves
        /// [`print_secret_export`], which prints it to stderr and puts it on
        /// no value this crate serialises anywhere.
        client_id: String,
    },
}

/// One provider's leg of an `ivar mcp auth` run — one entry per provider
/// (`R-ALL-PARTIAL`), never collapsed into a single flat result, so a
/// two-provider `--all-providers` run keeps each provider's own
/// pre-registration outcome and its own command line.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderRun {
    /// The provider this leg ran against.
    pub provider: Provider,
    /// What step 2 (pre-registration) did for this provider, if anything.
    pub preregistration: Preregistration,
    /// The harness's own command that ran for this provider — the whole of
    /// step 3, rendered exactly as [`proc::Command::display`] would show it
    /// in an error. Empty when step 2 itself failed and step 3 never ran.
    pub command: String,
    /// Whether this provider's harness reported success (exit 0). `false`
    /// means this leg failed — at pre-registration or at dispatch — and
    /// [`Self::error`] says what.
    pub authenticated: bool,
    /// What went wrong, when `authenticated` is `false`. Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What `ivar mcp auth` did: one [`ProviderRun`] per provider attempted.
#[derive(Debug, Clone, Serialize)]
pub struct AuthOutcome {
    /// The server that was authenticated.
    pub server: String,
    /// One entry per provider attempted — a single-element list for the
    /// default and `--provider` forms, one per hall provider for
    /// `--all-providers`.
    pub runs: Vec<ProviderRun>,
}

impl WriteHuman for AuthOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        for run in &self.runs {
            run.write_human(&self.server, w)?;
        }

        // A summary line only earns its place once there is more than one
        // leg to summarise — the single-provider form already said
        // everything worth saying above.
        if self.runs.len() > 1 {
            let succeeded: Vec<&str> = self
                .runs
                .iter()
                .filter(|run| run.authenticated)
                .map(|run| run.provider.id())
                .collect();
            let failed: Vec<&str> = self
                .runs
                .iter()
                .filter(|run| !run.authenticated)
                .map(|run| run.provider.id())
                .collect();

            if failed.is_empty() {
                writeln!(w, "All providers authenticated: {}.", succeeded.join(", "))?;
            } else {
                let succeeded = if succeeded.is_empty() {
                    "(none)".to_owned()
                } else {
                    succeeded.join(", ")
                };
                writeln!(
                    w,
                    "Succeeded: {succeeded} — Failed: {}. This run did not fully succeed.",
                    failed.join(", ")
                )?;
            }
        }

        Ok(())
    }
}

impl ProviderRun {
    /// One leg's lines, in [`AuthOutcome::write_human`]'s loop.
    fn write_human(&self, server: &str, w: &mut impl io::Write) -> io::Result<()> {
        let provider = self.provider;
        match &self.preregistration {
            Preregistration::NotNeeded => {}
            Preregistration::Skipped => writeln!(
                w,
                "[{provider}] a client registration for `{server}` was already on file — \
                 pre-registration skipped."
            )?,
            Preregistration::Registered { client_id } => writeln!(
                w,
                "[{provider}] pre-registered an OAuth client (`{client_id}`) for `{server}` with \
                 Figma. That only clears the registration allowlist — it is not authentication."
            )?,
        }

        if self.authenticated {
            writeln!(
                w,
                "[{provider}] authenticated `{server}` — `{}` exited 0.",
                self.command
            )
        } else {
            writeln!(
                w,
                "[{provider}] failed to authenticate `{server}`: {}",
                self.error.as_deref().unwrap_or("unknown error")
            )
        }
    }
}

/// Authenticate `input.server` under `input.provider`, the hall's default, or
/// — with `input.all_providers` — every provider the hall lists.
///
/// See the module doc comment for the steps, the sequential/partial-failure
/// contract of `--all-providers`, and what each failure mode means.
pub fn auth(ctx: &Ctx, input: AuthInput) -> Outcome<AuthOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    let server = resolve_server(&manifest, &input.server)?;

    if input.all_providers {
        // Sequential on purpose (`R-ALL-SEQUENTIAL`): `.map` over an
        // iterator, not a spawned task per provider — the next provider's
        // `proc::inherit` does not start until this one returns. Order
        // follows `providers.available` exactly, since `.map`/`.collect`
        // never reorders.
        let runs: Vec<ProviderRun> = manifest
            .providers()
            .available()
            .iter()
            .map(|&provider| run_provider(&layout, &manifest, server, provider))
            .collect();

        return Ok(all_providers_report(&server.name, runs));
    }

    let provider = resolve_provider(&manifest, input.provider.as_deref())?;
    let run = try_run_provider(&layout, &manifest, server, provider)?;
    Ok(Report::new(AuthOutcome {
        server: server.name.clone(),
        runs: vec![run],
    }))
}

/// Build the `--all-providers` report from every attempted [`ProviderRun`]:
/// one [`Warning`] per failed leg, never a [`Failure`] (`R-ALL-PARTIAL`) —
/// every provider was actually attempted, so a failed leg is "done, with
/// something needing attention" (exit `1`), not "refused" (exit `2`).
/// [`Report::is_clean`] turns `false` the moment any leg failed, which is
/// what keeps one success from ever rendering as the whole run succeeding.
fn all_providers_report(server: &str, runs: Vec<ProviderRun>) -> Report<AuthOutcome> {
    let warnings: Vec<Warning> = runs
        .iter()
        .filter(|run| !run.authenticated)
        .map(|run| {
            Warning::new(
                "mcp.provider_auth_failed",
                run.provider.id(),
                run.error.clone().unwrap_or_default(),
            )
        })
        .collect();

    Report::with_warnings(
        AuthOutcome {
            server: server.to_owned(),
            runs,
        },
        warnings,
    )
}

/// Find `name` in the manifest's declared MCP servers, or a [`Failure`]
/// listing every name that *is* declared.
fn resolve_server<'a>(manifest: &'a Manifest, name: &str) -> Result<&'a McpServerDef, Failure> {
    manifest
        .mcp_servers()
        .iter()
        .find(|server| server.name == name)
        .ok_or_else(|| {
            let declared: Vec<&str> = manifest
                .mcp_servers()
                .iter()
                .map(|server| server.name.as_str())
                .collect();
            let known = if declared.is_empty() {
                "(no servers declared in ivar.json's `mcp` array)".to_owned()
            } else {
                declared.join(", ")
            };
            Failure::blocked(
                "mcp.server_not_found",
                format!("no MCP server named `{name}` in ivar.json"),
            )
            .expected(format!("one of the declared servers: {known}"))
            .actual(format!("`{name}` is not declared"))
            .fix(FixAction::safe(
                "mcp.check_declared_servers",
                "Check the `mcp` array in ivar.json for the server's declared name.",
            ))
        })
}

/// `input.provider`, parsed; or the hall's default when omitted.
fn resolve_provider(manifest: &Manifest, raw: Option<&str>) -> Result<Provider, Failure> {
    match raw {
        Some(value) => value.parse::<Provider>().map_err(Failure::from),
        None => Ok(manifest.providers().default_provider()),
    }
}

/// Steps 2 and 3 for one provider, run exactly once. Both
/// [`try_run_provider`] (the single-provider path, which propagates a
/// failure immediately) and [`run_provider`] (`--all-providers`, which never
/// propagates) are thin wrappers over this — the only difference between the
/// two callers is what they do with [`Attempt::outcome`].
struct Attempt {
    preregistration: Preregistration,
    command: String,
    outcome: Result<(), Failure>,
}

fn attempt(
    layout: &Layout,
    manifest: &Manifest,
    server: &McpServerDef,
    provider: Provider,
) -> Attempt {
    let Preregistered {
        report: preregistration,
        fresh_secret,
    } = match preregister_if_needed(layout, manifest, provider, server) {
        Ok(preregistered) => preregistered,
        Err(failure) => {
            return Attempt {
                preregistration: Preregistration::NotNeeded,
                command: String::new(),
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
                outcome: Err(failure),
            };
        }
    };

    let command = auth_command(harness, &server.name, fresh_secret.as_ref());
    let display = command.display();
    let outcome = match proc::inherit(&command) {
        Ok(Some(0)) => verify_authenticated(harness, &server.name),
        Ok(code) => Err(login_failed(&display, code)),
        Err(spawn_error) => Err(spawn_error.into()),
    };

    Attempt {
        preregistration,
        command: display,
        outcome,
    }
}

/// The single-provider path: propagate [`Attempt::outcome`] immediately. A
/// dispatch failure here is still a hard [`Failure`] (exit `2`) — a single
/// explicit request that could not be completed is "broke mid-flight", the
/// same severity `action/sync/setup.rs` gives an inherited process's
/// non-zero exit.
fn try_run_provider(
    layout: &Layout,
    manifest: &Manifest,
    server: &McpServerDef,
    provider: Provider,
) -> Result<ProviderRun, Failure> {
    let Attempt {
        preregistration,
        command,
        outcome,
    } = attempt(layout, manifest, server, provider);
    outcome?;
    Ok(ProviderRun {
        provider,
        preregistration,
        command,
        authenticated: true,
        error: None,
    })
}

/// `--all-providers`'s path: never propagate. Every attempt becomes a
/// [`ProviderRun`] — success or failure — so the loop in [`auth`] keeps going
/// to the next provider regardless (`R-ALL-PARTIAL`).
fn run_provider(
    layout: &Layout,
    manifest: &Manifest,
    server: &McpServerDef,
    provider: Provider,
) -> ProviderRun {
    let Attempt {
        preregistration,
        command,
        outcome,
    } = attempt(layout, manifest, server, provider);
    match outcome {
        Ok(()) => ProviderRun {
            provider,
            preregistration,
            command,
            authenticated: true,
            error: None,
        },
        Err(failure) => ProviderRun {
            provider,
            preregistration,
            command,
            authenticated: false,
            error: Some(failure.what),
        },
    }
}

/// What step 2 did, plus — only for a fresh registration — the secret the
/// dispatched child needs in its own environment (defect fix, see the module
/// doc comment's "The secret must reach the very run that mints it" section).
///
/// Deliberately not `Serialize`: unlike [`Preregistration`], this type must
/// never become reachable from [`AuthOutcome`] or a `--json` run would print
/// the secret verbatim.
#[derive(Debug)]
struct Preregistered {
    /// What [`ProviderRun`] reports for step 2.
    report: Preregistration,
    /// `(env var name, secret value)`, present only when this call just
    /// registered a brand-new client. `None` for `NotNeeded` and `Skipped` —
    /// in both cases `ivar` never held a secret to hand off.
    fresh_secret: Option<(String, String)>,
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
fn preregister_if_needed(
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

/// Step 3's exit-0 answer is not enough to believe on every provider
/// (defect fix, `R-HONEST` — see the module doc comment). OpenCode's own
/// `opencode mcp auth` exits `0` unconditionally, so this checks the thing
/// itself: whether a token exchange actually landed in OpenCode's own
/// store. Claude Code's exit status, already checked by [`attempt`] before
/// this runs, is reliable — this is a no-op for it.
fn verify_authenticated(harness: Harness, server_name: &str) -> Result<(), Failure> {
    match harness {
        Harness::ClaudeCode => Ok(()),
        Harness::OpenCode => {
            if opencode_auth::has_tokens(server_name)? {
                return Ok(());
            }
            Err(Failure::failed(
                "mcp.auth_not_verified",
                format!(
                    "`opencode mcp auth {server_name}` exited 0, but no tokens for \
                     `{server_name}` were found in OpenCode's own credential store"
                ),
            )
            .expected("a `tokens` entry for this server in OpenCode's mcp-auth.json")
            .actual(
                "no tokens present — `opencode mcp auth` exits 0 even when it prints \
                 `Authentication failed` (measured 2026-08-26)",
            )
            .fix(FixAction::safe(
                "mcp.retry_auth",
                "Read the command's output above, then run `ivar mcp auth` again.",
            )))
        }
    }
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

/// The harness's own login command for `name` — the whole of step 3.
///
/// `fresh_secret`, when `Some((var, value))`, is set on the child's own
/// environment (defect fix, `R-SECRET-HANDOFF`): a client registered on this
/// very run needs its secret before the operator could possibly have
/// exported it. `None` on the `Skipped`/`NotNeeded` paths, where the
/// operator's own exported variable is the (only) source.
fn auth_command(
    harness: Harness,
    name: &str,
    fresh_secret: Option<&(String, String)>,
) -> proc::Command {
    let args: [&str; 2] = match harness {
        Harness::ClaudeCode => ["mcp", "login"],
        Harness::OpenCode => ["mcp", "auth"],
    };
    let command = proc::Command::new(harness.binary()).args(args).arg(name);
    match fresh_secret {
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
#[path = "../../../tests/unit/action/mcp/auth.rs"]
mod tests;
