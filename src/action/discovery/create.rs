//! `ivar discovery create <name>` — start a unit of work's memory.
//!
//! Writes `docs/<name>/discovery.md` with front matter in `exploring` and
//! an empty body, and creates `docs/<name>/research/` alongside it.
//!
//! No feature is required, and none is created: D3 says the order does not
//! matter, and discovery-then-feature is the normal one.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::discovery::DiscoveryDoc;
use crate::domain::name::FeatureName;
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;
use crate::store::discovery;

use super::super::discover_hall;

/// What `ivar discovery create` needs.
#[derive(Debug, Clone)]
pub struct CreateInput {
    /// The unit of work's name.
    pub name: String,
    /// A human-readable title. Defaults to the name.
    pub title: Option<String>,
}

/// What `ivar discovery create` did.
#[derive(Debug, Clone, Serialize)]
pub struct CreateOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The unit of work's name.
    pub name: FeatureName,
    /// `<hall>/docs/<name>/`.
    pub work_dir: Utf8PathBuf,
    /// `<hall>/docs/<name>/discovery.md`.
    pub doc: Utf8PathBuf,
}

impl WriteHuman for CreateOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Started discovery `{}`. Doc: {}", self.name, self.doc)
    }
}

/// Start a discovery.
///
/// # Errors
///
/// When no hall is found, when `name` is not a valid work name, or when a
/// discovery doc already exists for it.
pub fn create(ctx: &Ctx, input: CreateInput) -> Outcome<CreateOutcome> {
    let layout = discover_hall(ctx)?;
    let name = FeatureName::new(input.name)?;

    let doc_path = layout.discovery_doc(&name);
    if fs::is_file(&doc_path)? {
        return Err(Failure::blocked(
            "discovery.already_exists",
            format!("`{name}` already has a discovery doc"),
        )
        .expected("a name with no discovery doc yet")
        .actual(format!("`{doc_path}` already exists"))
        .fix(FixAction::safe(
            "discovery.amend_instead",
            format!("Add to it with `ivar discovery amend {name}`."),
        )));
    }

    let work_dir = layout.work_dir(&name);
    fs::ensure_dir(&work_dir)?;
    fs::ensure_dir(&layout.research_dir(&name))?;

    let doc = DiscoveryDoc::new(&name, input.title.as_deref(), &rfc3339_now());
    fs::write_text(&doc_path, &discovery::render(&doc)?)?;

    Ok(Report::new(CreateOutcome {
        root: layout.root().to_path_buf(),
        name,
        work_dir,
        doc: doc_path,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/discovery/create.rs"]
mod tests;
