//! The provider half of `ivar sync`: the canonical hall instructions and the
//! root aliases, the MCP config, and the shipped workflow commands —
//! materialised for a provider the hall lists, stripped for one it does not.
//!
//! The instruction topology is reconciled **once** per run through
//! [`harness::config::instructions`] — `HALL.md` plus every provider alias —
//! before the per-provider MCP/command loop. A conflict never aborts the
//! loop: the entry becomes a `Failed` line and an adoption warning, and the
//! rest of the provider work still lands.

use crate::domain::provider::Provider;
use crate::error::Warning;
use crate::harness::config::instructions;
use crate::harness::{commands, config};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::{Change, Entry, record_failure, repo_names};

pub(crate) fn sync_providers(
    layout: &Layout,
    manifest: &Manifest,
    entries: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
) {
    sync_instructions(layout, manifest, entries, warnings);
    for provider in Provider::ALL {
        sync_mcp(layout, manifest, provider, entries, warnings);
        sync_commands(layout, manifest, provider, entries, warnings);
    }
}

/// Reconcile `HALL.md` and every provider alias through the shared
/// reconciler. Conflicts and failures become warnings — a hall whose
/// instructions could not be materialised is still a valid hall.
pub(crate) fn sync_instructions(
    layout: &Layout,
    manifest: &Manifest,
    entries: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
) {
    match reconcile_instructions(layout, manifest) {
        Ok(changes) => {
            for change in changes {
                record_instruction_entry(&change, entries, warnings);
            }
        }
        Err(warning) => {
            entries.push(Entry::new("hall", "hall instructions", Change::Failed));
            warnings.push(warning);
        }
    }
}

/// Best-effort instruction bootstrap for `init` and `provider add`: the
/// reconciler runs once; any conflict or failure becomes a warning and never
/// rolls the command back. `ivar sync` is the repair.
pub(crate) fn materialise_instructions(
    layout: &Layout,
    manifest: &Manifest,
) -> Result<(), Warning> {
    match reconcile_instructions(layout, manifest) {
        Ok(changes) => {
            if let Some(conflict) = changes
                .into_iter()
                .find(|entry| entry.change == instructions::Change::Conflict)
            {
                return Err(adoption_warning(&conflict));
            }
            Ok(())
        }
        Err(warning) => Err(warning),
    }
}

/// The one call every instruction bootstrap shares: build the alias specs
/// from `Layout` and the manifest, and hand the reconciler the canonical path
/// and the managed block. A failure becomes a `not_materialised` warning.
fn reconcile_instructions(
    layout: &Layout,
    manifest: &Manifest,
) -> Result<Vec<instructions::Entry>, Warning> {
    let aliases = Provider::ALL.map(|provider| instructions::Alias {
        provider,
        path: layout.instruction_alias(&provider),
        enabled: manifest.providers().available().contains(&provider),
    });
    let block = config::build_block(manifest.name(), &repo_names(manifest));
    instructions::reconcile(&layout.hall_instructions(), &block, &aliases).map_err(|error| {
        Warning::new(
            "instructions.not_materialised",
            "hall",
            format!(
                "hall instructions could not be materialised: {error}; run `ivar sync` to repair"
            ),
        )
    })
}

/// One reconciler entry as a sync report line. A conflict is a `Failed` entry
/// plus an adoption warning — nothing was written to the conflicting path.
fn record_instruction_entry(
    entry: &instructions::Entry,
    entries: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
) {
    let (surface, label) = instruction_surface_label(entry);
    if entry.change == instructions::Change::Conflict {
        entries.push(Entry::new(surface, label.clone(), Change::Failed));
        warnings.push(adoption_warning(entry));
        return;
    }
    entries.push(Entry::new(surface, label, entry.change.into()));
}

/// The sync surface and label for a root instruction entry: the canonical
/// file belongs to `hall`; an alias belongs to its provider.
fn instruction_surface_label(entry: &instructions::Entry) -> (String, String) {
    let name = entry.path.file_name().unwrap_or("instructions").to_owned();
    let provider = Provider::ALL
        .iter()
        .find(|provider| provider.instruction_file() == name);
    match provider {
        Some(provider) => (provider.id().to_owned(), format!("{name} alias")),
        None => ("hall".to_owned(), name),
    }
}

/// The warning that names a conflicting instruction entry and the way
/// forward: consolidate into `HALL.md`, remove it, rerun sync, review the
/// diff. For the canonical file it names the regular-file requirement.
fn adoption_warning(entry: &instructions::Entry) -> Warning {
    let (surface, label) = instruction_surface_label(entry);
    let detail = entry.detail.clone().unwrap_or_default();
    Warning::new(
        "instructions.adoption_required",
        surface,
        format!("{label} needs a decision: {detail}"),
    )
}

pub(crate) fn sync_mcp(
    layout: &Layout,
    manifest: &Manifest,
    provider: Provider,
    entries: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
) {
    let path = layout.mcp_config(&provider);
    let label = format!("{} MCP config", provider.mcp_config_path());

    let result = if manifest.providers().available().contains(&provider) {
        config::materialise_mcp(&path, provider, manifest.mcp_servers())
    } else {
        config::remove_mcp(&path, provider)
    };

    match result {
        Ok(change) => entries.push(Entry::new(provider.id(), label, change.into())),
        Err(error) => record_failure(entries, warnings, provider.id(), &label, error.into()),
    }
}

pub(crate) fn sync_commands(
    layout: &Layout,
    manifest: &Manifest,
    provider: Provider,
    entries: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
) {
    let path = layout.commands_dir(&provider);
    let result = if manifest.providers().available().contains(&provider) {
        commands::materialise(&path)
    } else {
        commands::remove(&path)
    };

    match result {
        Ok(changes) => {
            for change in changes {
                entries.push(Entry::new(
                    provider.id(),
                    format!("command {}", change.file_name),
                    change.change.into(),
                ));
            }
        }
        Err(error) => {
            record_failure(
                entries,
                warnings,
                provider.id(),
                "official commands",
                error.into(),
            );
        }
    }
}

pub(crate) fn materialise_commands(layout: &Layout, provider: Provider) -> Result<(), Warning> {
    commands::materialise(&layout.commands_dir(&provider))
        .map(|_| ())
        .map_err(|error| {
            Warning::new(
                "provider.commands_not_materialised",
                provider.id(),
                format!(
                    "official commands could not be written: {error}; run `ivar sync` to repair"
                ),
            )
        })
}
