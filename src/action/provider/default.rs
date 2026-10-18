//! `ivar provider default <name>` — make a registered provider the hall's
//! default in `ivar.json`.
//!
//! Refuses an unknown id or a provider that is not registered before writing
//! anything. Naming the current default is a no-op that leaves the manifest
//! byte-for-byte unchanged.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::provider::Provider;
use crate::error::{Outcome, Report, WriteHuman};
use crate::store::manifest::{Manifest, Providers};

use super::super::{discover_hall, read_manifest};

/// What `ivar provider default` needs.
#[derive(Debug, Clone)]
pub struct DefaultInput {
    /// The provider's id.
    pub name: String,
}

/// What `ivar provider default` did.
#[derive(Debug, Clone, Serialize)]
pub struct DefaultOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The hall's default provider after the command.
    pub default: Provider,
    /// The hall's default provider before the command.
    pub previous: Provider,
    /// Whether `ivar.json` was rewritten.
    pub changed: bool,
}

impl WriteHuman for DefaultOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.changed {
            writeln!(
                w,
                "Default provider is now `{}` (was `{}`).",
                self.default, self.previous
            )
        } else {
            writeln!(w, "`{}` is already the default provider.", self.default)
        }
    }
}

/// Make `input.name` the hall's default provider.
pub fn set_default(ctx: &Ctx, input: &DefaultInput) -> Outcome<DefaultOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let provider: Provider = input.name.parse()?;
    let providers = manifest.providers();
    if !providers.available().contains(&provider) {
        return Err(super::not_available(provider));
    }
    let previous = providers.default_provider();
    let changed = previous != provider;
    if changed {
        let updated =
            manifest.with_providers(Providers::new(providers.available().to_vec(), provider))?;
        Manifest::write(&layout, &updated)?;
    }
    Ok(Report::new(DefaultOutcome {
        root: layout.root().to_path_buf(),
        default: provider,
        previous,
        changed,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/provider/default.rs"]
mod tests;
