//! The provider half of `ivar sync`: the managed block, the MCP config, and
//! the shipped workflow commands — materialised for a provider the hall lists,
//! stripped for one it does not.

use crate::domain::provider::Provider;
use crate::error::Warning;
use crate::harness::{commands, config};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::{Entry, record_failure, repo_names};

pub(crate) fn sync_providers(
    layout: &Layout,
    manifest: &Manifest,
    entries: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
) {
    let block = config::build_block(manifest.name(), &repo_names(manifest));
    for provider in Provider::ALL {
        sync_provider(layout, manifest, provider, &block, entries, warnings);
        sync_mcp(layout, manifest, provider, entries, warnings);
        sync_commands(layout, manifest, provider, entries, warnings);
    }
}

pub(crate) fn sync_provider(
    layout: &Layout,
    manifest: &Manifest,
    provider: Provider,
    block: &str,
    entries: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
) {
    let path = layout.instruction_file(&provider);
    let label = format!("{} managed block", provider.instruction_file());

    let result = if manifest.providers().available().contains(&provider) {
        config::materialise(&path, block)
    } else {
        config::remove(&path)
    };

    match result {
        Ok(change) => entries.push(Entry::new(provider.id(), label, change.into())),
        Err(error) => record_failure(entries, warnings, provider.id(), &label, error.into()),
    }
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
