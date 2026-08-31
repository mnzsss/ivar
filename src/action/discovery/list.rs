//! `ivar discovery list` — every unit of work with committed memory.
//!
//! Scans `<hall>/docs/` and keeps a child only when its name is a valid
//! work name *and* it holds a `discovery.md`. That pair of conditions is
//! D11: `docs/product/`, `docs/updates/`, and `docs/repo-relations/` are
//! the hall's own topic documentation, and every other folder there is the
//! team's. ivar reports only what it created.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::discovery::DiscoveryStatus;
use crate::domain::name::FeatureName;
use crate::error::{Outcome, Report, WriteHuman};
use crate::infra::fs;
use crate::store::discovery;

use super::super::discover_hall;

/// What `ivar discovery list` needs.
#[derive(Debug, Clone)]
pub struct ListInput {
    /// Report only discoveries in this status. `None` reports all.
    pub status: Option<DiscoveryStatus>,
}

/// One discovery, as listed.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    /// The unit of work's name.
    pub name: FeatureName,
    /// Its title, or the name when the header could not be read.
    pub title: String,
    /// Where it stands.
    pub status: DiscoveryStatus,
}

/// What `ivar discovery list` found.
#[derive(Debug, Clone, Serialize)]
pub struct ListOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// One entry per discovery, sorted by name.
    pub discoveries: Vec<Summary>,
}

impl WriteHuman for ListOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.discoveries.is_empty() {
            writeln!(w, "No discoveries in {}.", self.root)?;
            return Ok(());
        }
        writeln!(w, "Discoveries in {}:", self.root)?;
        for entry in &self.discoveries {
            writeln!(
                w,
                "  {}  [{}]  {}",
                entry.name,
                entry.status.as_str(),
                entry.title
            )?;
        }
        Ok(())
    }
}

/// List every unit of work with committed memory.
///
/// # Errors
///
/// When no hall is found, or `<hall>/docs/` cannot be read.
pub fn list(ctx: &Ctx, input: ListInput) -> Outcome<ListOutcome> {
    let layout = discover_hall(ctx)?;
    let docs_root = layout.work_docs_root();

    let mut discoveries = Vec::new();
    if fs::is_dir(&docs_root)? {
        for child in fs::read_dir(&docs_root)? {
            if !fs::is_dir(&child)? {
                continue;
            }
            // The name is the gate. A folder ivar could not have created is
            // not ivar's to report — see the module doc.
            let Some(basename) = child.file_name() else {
                continue;
            };
            let Ok(name) = FeatureName::new(basename) else {
                continue;
            };
            let doc_path = layout.discovery_doc(&name);
            if !fs::is_file(&doc_path)? {
                continue;
            }

            let doc = discovery::parse(&fs::read_text(&doc_path)?.unwrap_or_default());
            let title = if doc.frontmatter.title.is_empty() {
                name.as_str().to_owned()
            } else {
                doc.frontmatter.title.clone()
            };
            discoveries.push(Summary {
                name,
                title,
                status: doc.frontmatter.status,
            });
        }
    }

    if let Some(wanted) = input.status {
        discoveries.retain(|entry| entry.status == wanted);
    }
    discoveries.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));

    Ok(Report::new(ListOutcome {
        root: layout.root().to_path_buf(),
        discoveries,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/discovery/list.rs"]
mod tests;
