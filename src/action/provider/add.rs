//! `ivar provider add <name>` — register a provider in `ivar.json`.
//!
//! Appends `name` to `providers.available`, leaving `providers.default`
//! untouched — adding a harness never silently changes which one sessions
//! start in. The manifest is the committed record, and the new provider's
//! shipped workflow commands are materialised immediately; `ivar sync` is what
//! materialises the rest of its config (managed block, MCP) from the manifest.
//!
//! Two refusals, both before anything is written: an unknown provider id
//! (the set is closed — `claude-code` and `opencode`), and a provider that is
//! already registered. Neither touches `ivar.json`.
//!
//! A command-write failure is a **warning**, not a failure: the manifest keeps
//! the provider, and the warning names `ivar sync` as the repair.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::store::manifest::{Manifest, Providers};

use super::super::sync;
use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// What `ivar provider add` needs.
#[derive(Debug, Clone)]
pub struct AddInput {
    /// The provider's id — `claude-code` or `opencode`.
    pub name: String,
}

/// What `ivar provider add` did.
#[derive(Debug, Clone, Serialize)]
pub struct AddOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The provider that was registered.
    pub provider: Provider,
    /// Every provider the hall now lists, in id order.
    pub available: Vec<Provider>,
    /// The hall's default provider — unchanged by this command.
    pub default: Provider,
    /// Number of skill targets changed during reconciliation.
    pub skill_targets: u64,
}

impl WriteHuman for AddOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let rendered = self
            .available
            .iter()
            .map(Provider::id)
            .collect::<Vec<_>>()
            .join(", ");
        write!(
            w,
            "Registered provider `{}` in {} — available: {} (default: {}).",
            self.provider, self.root, rendered, self.default
        )?;
        if self.skill_targets > 0 {
            write!(w, " Skill targets changed: {}.", self.skill_targets)?;
        }
        writeln!(w)
    }
}

/// Register `input.name` in the hall's `providers.available`.
///
/// Blocked when the provider id is unknown or already registered — in both
/// cases `ivar.json` is left untouched.
pub fn add(ctx: &Ctx, input: &AddInput) -> Outcome<AddOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    // Unknown providers are refused — the set is closed, and a typo must not
    // become a silent no-op or a new, unbuildable harness.
    let provider: Provider = input.name.parse()?;

    // Duplicates are refused, never silently collapsed.
    if manifest.providers().available().contains(&provider) {
        return Err(Failure::blocked(
            "provider.already_available",
            format!("`{provider}` is already registered in ivar.json"),
        )
        .expected("a provider not yet in `providers.available`")
        .actual(format!("`{provider}` is already listed"))
        .fix(FixAction::safe(
            "provider.use_existing",
            format!("Nothing to do — `{provider}` is already available."),
        )));
    }

    let default = manifest.providers().default_provider();
    let mut available = manifest.providers().available().to_vec();
    available.push(provider);
    available.sort();

    // Rebuild the manifest through the manifest's own `with_providers`, which
    // preserves the hall integration defaults, MCP servers, and repos.
    let updated = manifest.with_providers(Providers::new(available.clone(), default))?;
    Manifest::write(&layout, &updated)?;

    // The provider is registered and its config is bootstrapped in the same
    // run — no follow-up sync needed. The new provider's root alias and
    // shipped workflow commands are materialised immediately; a conflict or
    // write failure is a warning (the manifest keeps the provider; sync is
    // the repair), never a rollback.
    let mut skill_warnings = Vec::new();
    let skill_targets =
        match crate::action::skill::sync::reconcile(&layout, &updated, &mut skill_warnings) {
            Ok(count) => count,
            Err(failure) => {
                skill_warnings.push(Warning::new(
                    "provider.skill_reconcile_failed",
                    provider.id(),
                    failure.to_string(),
                ));
                0
            }
        };
    let mut report = Report::new(AddOutcome {
        root: layout.root().to_path_buf(),
        provider,
        available,
        default,
        skill_targets,
    });
    for warning in sync::materialise_instructions(&layout, &updated) {
        report.warn(warning);
    }
    if let Some(warning) = sync::materialise_commands(&layout, provider) {
        report.warn(warning);
    }
    if let Some(warning) = sync::materialise_shipped_skills(&layout, provider) {
        report.warn(warning);
    }
    for warning in skill_warnings {
        report.warn(warning);
    }
    Ok(report)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/provider/add.rs"]
mod tests;
