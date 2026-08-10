//! `ivar provider list` — the harnesses a hall knows about, and the default.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::provider::Provider;
use crate::error::{Outcome, Report, WriteHuman};

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// What `ivar provider list` found.
#[derive(Debug, Clone, Serialize)]
pub struct ListOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// Every provider the hall lists.
    pub available: Vec<Provider>,
    /// The provider `ivar session start` picks when none is named.
    pub default: Provider,
}

impl WriteHuman for ListOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let rendered = self
            .available
            .iter()
            .map(Provider::id)
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            w,
            "Providers in {}: {} (default: {})",
            self.root,
            rendered,
            self.default.id()
        )
    }
}

/// List the hall's providers from `ivar.json`.
pub fn list(ctx: &Ctx) -> Outcome<ListOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    Ok(Report::new(ListOutcome {
        root: layout.root().to_path_buf(),
        available: manifest.providers().available().to_vec(),
        default: manifest.providers().default_provider(),
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/provider/list.rs"]
mod tests;
