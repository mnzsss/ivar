//! `ivar skill detach <skill>` — convert an external skill to an authored one.
//!
//! Drops the `source` field from the SKILL.md frontmatter while preserving the
//! body text, byte-for-byte. An already-authored skill is a no-op (explained).
//! No network access.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::{Ctx, Done};
use crate::domain::name::RepoName;
use crate::domain::skill::Source;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::frontmatter;
use crate::infra::fs;
use crate::store::skill;

use super::super::discover_hall;

#[derive(Debug, Clone)]
pub struct DetachInput {
    pub skill: String,
}

/// What `ivar skill detach` did.
#[derive(Debug, Clone, Serialize)]
pub struct DetachOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The skill's id that was detached.
    pub id: RepoName,
}

impl WriteHuman for DetachOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Detached skill `{}` — now authored locally.", self.id)
    }
}

pub fn detach(ctx: &Ctx, input: DetachInput) -> Outcome<Done> {
    let layout = discover_hall(ctx)?;
    let skills_dir = layout.hall_skills();

    // Find and parse the skill.
    let skill_dir = skills_dir.join(&input.skill);
    if !fs::exists(&skill_dir)? {
        return Err(Failure::blocked(
            "skill.not_found",
            format!("skill `{}` does not exist", input.skill),
        )
        .expected("a skill directory under `.ivar/skills/`")
        .actual(format!("no directory at `{skill_dir}`"))
        .fix(FixAction::safe(
            "skill.list",
            "List available skills to find the correct id.",
        )));
    }

    let Some(skill) = skill::parse_skill(skill_dir.clone()).map_err(|e| {
        Failure::failed(
            "skill.parse_error",
            format!("could not parse skill `{}`: {}", input.skill, e),
        )
    })?
    else {
        return Err(Failure::blocked(
            "skill.no_frontmatter",
            format!("skill `{}` has no valid frontmatter", input.skill),
        ));
    };

    // Detach of an authored skill is a no-op (explained).
    if matches!(&skill.source, Source::Authored) {
        return Ok(Report::new(Done));
    }

    // Read the current SKILL.md.
    let raw = fs::read_text(&skill_dir.join("SKILL.md"))
        .map_err(|e| Failure::failed("skill.read_error", format!("could not read SKILL.md: {e}")))?
        .ok_or_else(|| {
            Failure::failed(
                "skill.missing_file",
                "SKILL.md not found in skill directory".to_owned(),
            )
        })?;

    // Parse frontmatter, strip the source field, re-emit with body untouched.
    let fm = skill::parse_frontmatter(&raw)?.ok_or_else(|| {
        Failure::failed(
            "skill.no_frontmatter",
            "SKILL.md has no parseable frontmatter".to_owned(),
        )
    })?;

    // Detached frontmatter: same name and description, but no source.
    let detached_fm = crate::domain::skill::SkillFrontmatter {
        name: fm.name,
        description: fm.description,
        source: None, // Drop the source — this is the detach.
    };

    // Re-emit with frontmatter replaced, body byte-for-byte identical.
    let new_content = frontmatter::replace(&raw, &detached_fm).map_err(|e| {
        Failure::failed(
            "skill.serialize_error",
            format!("could not serialize frontmatter: {e}"),
        )
    })?;

    fs::write_text(&skill_dir.join("SKILL.md"), &new_content).map_err(|e| {
        Failure::failed(
            "skill.write_error",
            format!("could not update SKILL.md: {e}"),
        )
    })?;

    Ok(Report::new(Done))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/skill/detach.rs"]
mod tests;
