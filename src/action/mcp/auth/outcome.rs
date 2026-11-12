use std::io;

use serde::Serialize;

use crate::domain::provider::Provider;
use crate::error::{Report, Warning, WriteHuman};

/// Whether step 2 (pre-registration) ran, and what it found.
///
/// Never conflated with authentication — see the module doc comment's
/// `R-HONEST` section. [`WriteHuman`] gives each variant its own sentence so a
/// reader cannot mistake "a client is registered" for "this server is
/// authenticated".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Preregistration {
    /// The server needs no authentication at all: it served `initialize`
    /// without a challenge, so there is nothing to register and nothing to
    /// authenticate.
    NoAuthRequired,
    /// Step 2 never applied: the provider owns the login, the server has no
    /// `url`, or it advertises neither a client metadata document nor a
    /// `registration_endpoint`.
    NotNeeded,
    /// A usable client registration was already on the manifest; nothing was
    /// registered or touched (`R-IDEMPOTENT`).
    Skipped,
    /// A new client was registered (Figma's registrar or the server's
    /// `registration_endpoint`), written into `ivar.json` and materialised
    /// into the provider config.
    Registered {
        /// The `client_id` the server issued. Not a secret by itself, but never
        /// reported alongside the client secret either — which is saved
        /// locally to `.ivar/secrets/mcp.env` and passed to the child process environment.
        client_id: String,
    },
    /// The authorization server accepts Client ID Metadata Documents: ivar
    /// identified itself by the URL of its published document, registered
    /// nothing, and recorded it in `ivar.json` as a public client.
    MetadataDocument { client_id: String },
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
    /// How authentication was performed.
    pub auth_method: AuthMethod,
    /// The harness's own command that ran for this provider — the whole of
    /// step 3, rendered exactly as
    /// [`proc::Command::display`](crate::infra::proc::Command::display) would
    /// show it in an error. Empty when step 2 itself failed and step 3 never ran.
    /// For [`AuthMethod::InternalOAuthFlow`], contains a descriptive label
    /// instead of a command line.
    pub command: String,
    /// Whether this provider's harness reported success (exit 0). `false`
    /// means this leg failed — at pre-registration or at dispatch — and
    /// [`Self::error`] says what.
    pub authenticated: bool,
    /// What went wrong, when `authenticated` is `false`. Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// How authentication was performed for a provider run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    /// The harness's own login command (`claude mcp login` / `opencode mcp auth`)
    /// was spawned via `proc::inherit`.
    ProviderCommand,
    /// Ivar performed the OAuth authorization-code flow itself, printing
    /// the authorization URL and running a temporary loopback listener.
    InternalOAuthFlow,
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
            Preregistration::MetadataDocument { client_id } => writeln!(
                w,
                "[{provider}] `{server}` accepts a client metadata document — ivar identifies \
                 itself as `{client_id}`; nothing was registered."
            )?,
            Preregistration::NoAuthRequired => writeln!(
                w,
                "[{provider}] `{server}` needs no authentication — it answers without a \
                 challenge."
            )?,
        }

        if self.authenticated {
            match self.auth_method {
                AuthMethod::ProviderCommand => writeln!(
                    w,
                    "[{provider}] authenticated `{server}` — `{}` exited 0.",
                    self.command
                ),
                AuthMethod::InternalOAuthFlow => writeln!(
                    w,
                    "[{provider}] authenticated `{server}` via Ivar's OAuth flow.",
                ),
            }
        } else {
            writeln!(
                w,
                "[{provider}] failed to authenticate `{server}`: {}",
                self.error.as_deref().unwrap_or("unknown error")
            )
        }
    }
}

/// Build the `--all-providers` report from every attempted [`ProviderRun`]:
/// one [`Warning`] per failed leg, never a
/// [`Failure`](crate::error::Failure) (`R-ALL-PARTIAL`) — every provider was
/// actually attempted, so a failed leg is "done, with something needing
/// attention" (exit `1`), not "refused" (exit `2`).
/// [`Report::is_clean`] turns `false` the moment any leg failed, which is
/// what keeps one success from ever rendering as the whole run succeeding.
pub(super) fn all_providers_report(server: &str, runs: Vec<ProviderRun>) -> Report<AuthOutcome> {
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

pub(super) fn dropped_grants(
    server: &str,
    runs: &[ProviderRun],
    after: &std::collections::BTreeMap<Provider, crate::domain::mcp::CredentialState>,
) -> Vec<Warning> {
    let mut warnings = Vec::new();
    for run in runs {
        if run.authenticated
            && let Some(&state) = after.get(&run.provider)
            && state == crate::domain::mcp::CredentialState::Missing
        {
            let provider_id = run.provider.id();
            warnings.push(Warning::new(
                "mcp.grant_dropped",
                provider_id,
                format!(
                    "`{server}` lost its {provider_id} grant after later authorizations; \
                     run `ivar mcp auth {server} --provider {provider_id}`"
                ),
            ));
        }
    }
    warnings
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/mcp/auth/outcome.rs"]
mod tests;
