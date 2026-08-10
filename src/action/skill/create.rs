//! `ivar skill create <id> --description "..."` — scaffold a new skill.
//!
//! Creates `<hall>/.ivar/skills/<id>/SKILL.md` with `name` and `description`
//! frontmatter. Refuses when the skill already exists — a skill is a folder
//! of files a team may have grown; regenerating it would discard that work.

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
    /// The skill's id — one path segment, unique within the skills dir.
    pub id: String,
    /// The skill's description, for the `SKILL.md` frontmatter.
    pub description: String,
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

    let skill_dir = layout.hall_skills().join(id.as_str());
    let skill_file = skill_dir.join("SKILL.md");
    if fs::exists(&skill_file)? {
        return Err(Failure::blocked(
            "skill.already_exists",
            format!("skill `{id}` already exists"),
        )
        .expected("a skill id that has not been used before")
        .actual(format!("`{skill_file}` already exists"))
        .fix(FixAction::safe(
            "skill.use_existing",
            "Edit the existing skill, or remove it deliberately first.",
        )));
    }

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
