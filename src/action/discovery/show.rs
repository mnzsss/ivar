//! `ivar discovery show <name>` — print a unit of work's memory, or just
//! say where it lives.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::name::FeatureName;
use crate::error::{Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;

/// What `ivar discovery show` needs.
#[derive(Debug, Clone)]
pub struct ShowInput {
    /// The unit of work's name.
    pub name: String,
    /// Print only the path, not the content.
    pub path_only: bool,
}

/// What `ivar discovery show` found.
#[derive(Debug, Clone, Serialize)]
pub struct ShowOutcome {
    /// The doc's path.
    pub path: Utf8PathBuf,
    /// The doc's full text. `None` with `--path`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

impl WriteHuman for ShowOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        match &self.content {
            Some(content) => write!(w, "{content}"),
            None => writeln!(w, "{}", self.path),
        }
    }
}

/// Print a discovery doc.
///
/// # Errors
///
/// When no hall is found, `name` is not a valid work name, or the name has
/// no discovery doc.
pub fn show(ctx: &Ctx, input: ShowInput) -> Outcome<ShowOutcome> {
    let layout = discover_hall(ctx)?;
    let name = FeatureName::new(input.name)?;
    // Fails with `discovery.not_found` when absent — the shared message.
    super::load(&layout, &name)?;

    let path = layout.discovery_doc(&name);
    let content = if input.path_only {
        None
    } else {
        Some(fs::read_text(&path)?.unwrap_or_default())
    };

    Ok(Report::new(ShowOutcome { path, content }))
}
