//! `ivar skill add <repo> [--path] [--ref]` — install an external skill.
//!
//! Creates `<hall>/.ivar/skills/<id>/SKILL.md` with frontmatter recording the
//! upstream source (`repo`, `path`, `ref`). The skill's id is derived from the
//! last path segment of `repo` (e.g. `"owner/my-toolkit"` → `"my-toolkit"`).
//!
//! Refuses when a skill with that id already exists.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::domain::skill::ExternalRef;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;
use crate::action::Ctx;

/// What `ivar skill add` needs.
#[derive(Debug, Clone)]
pub struct AddInput {
    pub repo: String,
    pub path: Option<String>,
    pub ref_: Option<String>,
    /// Install into the committed hall root instead of the personal one.
    pub hall: bool,
}

/// What `ivar skill add` did.
#[derive(Debug, Clone, Serialize)]
pub struct AddOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The skill's id — derived from the repo name.
    pub id: RepoName,
    /// The path of the written `SKILL.md`.
    pub skill_file: Utf8PathBuf,
}

impl WriteHuman for AddOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Added skill `{}` from {} at {}",
            self.id, self.skill_file, self.root
        )
    }
}

/// Install an external skill named after the repo's last path segment.
pub fn add(ctx: &Ctx, input: AddInput) -> Outcome<AddOutcome> {
    let layout = discover_hall(ctx)?;

    // Derive the skill id from the repo name — the last path segment.
    let id_str = input.repo.rsplit('/').next().ok_or_else(|| {
        Failure::failed("skill.add.bad_repo", "repo must contain at least one '/'")
    })?;

    let id = RepoName::new(id_str)?;

    // Refuse against both roots — see `create` for why.
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

    let ext = ExternalRef {
        repo: input.repo.clone(),
        path: input.path.clone().unwrap_or_default(),
        git_ref: input.ref_.clone().unwrap_or_default(),
    };

    write_skill_markdown(&skill_file, &id, &ext)?;

    Ok(Report::new(AddOutcome {
        root: layout.root().to_path_buf(),
        id,
        skill_file,
    }))
}

/// Write the SKILL.md file with frontmatter recording the source.
fn write_skill_markdown(
    path: &camino::Utf8Path,
    id: &RepoName,
    ext: &ExternalRef,
) -> Result<(), Failure> {
    let lines: Vec<String> = vec![
        "---".to_owned(),
        format!("name: {id}"),
        format!("description: External skill from {}", ext.repo),
        "source:".to_owned(),
        format!("  repo: \"{}\"", ext.repo),
        format!("  path: \"{}\"", ext.path),
        format!("  ref: \"{}\"", ext.git_ref),
        "---".to_owned(),
        String::new(),
    ];
    let content = lines.join("\n");
    fs::write_text(path, &content).map_err(|e| e.into())
}

#[cfg(test)]
#[path = "../../../tests/unit/action/skill/add.rs"]
mod tests;
