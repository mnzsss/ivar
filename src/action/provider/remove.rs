//! `ivar provider remove <name>` — unregister a provider from `ivar.json`.
//!
//! Drops `name` from `providers.available` and tears down its ivar-owned
//! config through the same reconciliation `ivar sync` runs, so a removal
//! and a hand-edit followed by sync end in the same state.
//!
//! Every refusal happens before anything is written: an unknown id, a
//! provider that is not registered, the last provider, and removing the
//! default without naming a replacement via `--default`.
//!
//! Live sessions launched with the removed provider never block: each one
//! becomes a warning, because its `ivar guard` hook config is gone.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::action::session::lookup;
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers};

use super::super::sync;
use super::super::{discover_hall, read_manifest};

/// What `ivar provider remove` needs.
#[derive(Debug, Clone)]
pub struct RemoveInput {
    /// The provider's id.
    pub name: String,
    /// The provider to make the default in the same write.
    pub default: Option<String>,
}

/// What `ivar provider remove` did.
#[derive(Debug, Clone, Serialize)]
pub struct RemoveOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The provider that was removed.
    pub provider: Provider,
    /// Every provider the hall still lists, in id order.
    pub available: Vec<Provider>,
    /// The hall's default provider after the removal.
    pub default: Provider,
}

impl WriteHuman for RemoveOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let rendered = self
            .available
            .iter()
            .map(Provider::id)
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            w,
            "Removed provider `{}` from {} — available: {} (default: {}).",
            self.provider, self.root, rendered, self.default
        )
    }
}

/// Remove `input.name` from the hall's `providers.available` and tear down
/// its config.
pub fn remove(ctx: &Ctx, input: &RemoveInput) -> Outcome<RemoveOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let provider: Provider = input.name.parse()?;
    let requested_default: Option<Provider> =
        input.default.as_deref().map(str::parse).transpose()?;

    let providers = manifest.providers();
    if !providers.available().contains(&provider) {
        return Err(not_available(provider));
    }
    let available: Vec<Provider> = providers
        .available()
        .iter()
        .copied()
        .filter(|p| *p != provider)
        .collect();
    if available.is_empty() {
        return Err(last_provider(provider));
    }
    let default = match requested_default {
        Some(candidate) if available.contains(&candidate) => candidate,
        Some(candidate) => return Err(invalid_default(provider, candidate)),
        None if providers.default_provider() == provider => {
            return Err(default_required(provider, &available));
        }
        None => providers.default_provider(),
    };

    let updated = manifest.with_providers(Providers::new(available.clone(), default))?;
    let mut warnings = unguarded_sessions(&layout, provider);
    Manifest::write(&layout, &updated)?;

    let mut entries = Vec::new();
    sync::sync_providers(&layout, &updated, &mut entries, &mut warnings);

    Ok(Report::with_warnings(
        RemoveOutcome {
            root: layout.root().to_path_buf(),
            provider,
            available,
            default,
        },
        warnings,
    ))
}

fn not_available(provider: Provider) -> Failure {
    Failure::blocked(
        "provider.not_available",
        format!("`{provider}` is not registered in ivar.json"),
    )
    .expected("a provider listed in `providers.available`")
    .actual(format!("`{provider}` is not listed"))
    .fix(
        FixAction::safe("provider.list", "See the registered providers.")
            .command("ivar provider list"),
    )
}

fn last_provider(provider: Provider) -> Failure {
    Failure::blocked(
        "provider.last_provider",
        format!("`{provider}` is the hall's only provider"),
    )
    .expected("at least one provider left in `providers.available`")
    .actual(format!("removing `{provider}` would leave none"))
    .fix(
        FixAction::safe(
            "provider.add_first",
            "Register the replacement first, then remove this one.",
        )
        .command("ivar provider add <name>"),
    )
}

fn default_required(provider: Provider, remaining: &[Provider]) -> Failure {
    let replacement = remaining.first().copied().unwrap_or(provider);
    Failure::blocked(
        "provider.default_required",
        format!("`{provider}` is the hall's default provider"),
    )
    .expected("`--default <provider>` naming a provider that stays registered")
    .actual("no `--default` given")
    .fix(
        FixAction::safe(
            "provider.name_default",
            "Name the new default in the same command.",
        )
        .command(format!(
            "ivar provider remove {provider} --default {replacement}"
        )),
    )
}

fn invalid_default(provider: Provider, candidate: Provider) -> Failure {
    Failure::blocked(
        "provider.invalid_default",
        format!("`{candidate}` cannot become the default when removing `{provider}`"),
    )
    .expected("a registered provider other than the one being removed")
    .actual(format!(
        "`{candidate}` would not be in `providers.available`"
    ))
    .fix(
        FixAction::safe("provider.list", "See the registered providers.")
            .command("ivar provider list"),
    )
}

fn unguarded_sessions(layout: &Layout, removed: Provider) -> Vec<Warning> {
    let sessions = match lookup::list_all(layout) {
        Ok(sessions) => sessions,
        Err(failure) => {
            return vec![Warning::new(
                "provider.sessions_unchecked",
                removed.id(),
                format!(
                    "live sessions could not be listed ({}); any `{removed}` session still running no longer has `ivar guard` hook config",
                    failure.what
                ),
            )];
        }
    };
    sessions
        .into_iter()
        .filter(|session| session.state.as_ref().is_some_and(|state| state.provider() == removed))
        .map(|session| {
            Warning::new(
                "provider.session_unguarded",
                session.id.as_str(),
                format!(
                    "session `{}` was launched with `{removed}` and no longer has `ivar guard` hook config",
                    session.id
                ),
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/action/provider/remove.rs"]
mod tests;
