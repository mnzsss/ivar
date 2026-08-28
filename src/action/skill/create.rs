//! `ivar skill create <id> --description "..."` — scaffold a new skill.
//!
//! Writes `SKILL.md` with `name` and `description` frontmatter into the
//! personal root, `<hall>/.ivar/skills-local/<id>/`. `--hall` targets the
//! committed root instead.
//!
//! # Why personal is the default
//!
//! The committed root is shared: a skill written there is a commit, visible
//! in review, and inherited by everyone who clones the hall. Publishing is
//! the consequential act, so it is the one that gets a flag. A skill written
//! by reflex stays private, and promoting it is a deliberate second step.
//!
//! Refuses when the skill already exists **in either root** — regenerating
//! would discard work, and creating the twin of an existing id would build a
//! collision that `sync` then has to refuse.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;
use crate::action::Ctx;

/// What `ivar skill create` needs.
#[derive(Debug, Clone)]
pub struct CreateInput {
    /// The skill's id — one path segment, unique across both roots.
    pub id: String,
    /// The skill's description, for the `SKILL.md` frontmatter.
    pub description: String,
    /// Write to the committed hall root instead of the personal one.
    pub hall: bool,
}

/// What `ivar skill create` did.
#[derive(Debug, Clone, Serialize)]
pub struct CreateOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The skill's id.
    pub id: RepoName,
    /// The path of the created `SKILL.md`.
    pub skill_file: Utf8PathBuf,
}

impl WriteHuman for CreateOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Created skill `{}` at {}", self.id, self.skill_file)
    }
}

/// Scaffold a skill named `input.id` with `input.description`.
///
/// Blocked when a skill with that id already exists, and when the id is not
/// a valid repo-name segment (the same rules apply: it becomes a directory).
pub fn create(ctx: &Ctx, input: CreateInput) -> Outcome<CreateOutcome> {
    let layout = discover_hall(ctx)?;
    let id = RepoName::new(input.id)?;

    // Refuse against both roots, not just the one being written to: an id
    // that exists in the other root would become a collision on the next
    // sync, and refusing now is cheaper than diagnosing it later.
    if let Some((existing, _)) = super::enumerate::resolve(&layout, id.as_str())? {
        return Err(Failure::blocked(
            "skill.already_exists",
            format!("skill `{id}` already exists"),
        )
        .expected("a skill id that has not been used in either root")
        .actual(format!("`{existing}` already exists"))
        .fix(FixAction::safe(
            "skill.use_existing",
            "Edit the existing skill, or remove it deliberately first.",
        )));
    }

    let root_dir = if input.hall {
        layout.hall_skills()
    } else {
        layout.hall_skills_local()
    };
    let skill_dir = root_dir.join(id.as_str());
    let skill_file = skill_dir.join("SKILL.md");

    fs::ensure_dir(&skill_dir)?;
    fs::write_text(
        &skill_file,
        &format!(
            "---\nname: {id}\ndescription: {}\n---\n\n",
            input.description
        ),
    )?;

    Ok(Report::new(CreateOutcome {
        root: layout.root().to_path_buf(),
        id,
        skill_file,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/skill/create.rs"]
mod tests;
