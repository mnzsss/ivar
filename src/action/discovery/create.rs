//! `ivar discovery create <name>` — start a unit of work's memory.
//!
//! Writes `<hall>/.ivar/features/<name>/discovery.md` (or `<view_dir>/discovery.md`
//! inside an unconverted discovery session) with front matter in `exploring` and
//! an empty body.
//!
//! No feature is required, and none is created: D3 says the order does not
//! matter, and discovery-then-feature is the normal one.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::action::session::env::SessionEnv;
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
    /// `<hall>/.ivar/features/<name>/`.
    pub feature_dir: Utf8PathBuf,
    /// The path where the doc was written.
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

    let doc_path = super::resolve_doc_path(ctx, &layout, &name)?;
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

    if let Some(parent) = doc_path.parent() {
        fs::ensure_dir(parent)?;
    }

    let mut doc = DiscoveryDoc::new(&name, input.title.as_deref(), &rfc3339_now());
    // A doc created inside a discovery session must record that session in its
    // front matter. `session convert` resolves the name to promote by finding
    // the doc whose `sessions` lists the converting session; a doc that never
    // names its own session could never be converted, which is the entire path
    // a discovery session exists to take.
    if let Ok(Some(env)) = SessionEnv::resolve_by_cwd(&ctx.cwd)
        && env.feature.is_none()
    {
        doc.frontmatter.sessions.push(env.session_id);
    }
    fs::write_text(&doc_path, &discovery::render(&doc)?)?;

    Ok(Report::new(CreateOutcome {
        root: layout.root().to_path_buf(),
        name: name.clone(),
        feature_dir: layout.feature_dir(&name),
        doc: doc_path,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/discovery/create.rs"]
mod tests;
