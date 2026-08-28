//! `ivar skill list` — the skills the hall can use, from both roots.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::domain::skill::SkillRoot;
use crate::error::{Outcome, Report, WriteHuman};
use crate::infra::frontmatter;
use crate::infra::fs;

use super::super::discover_hall;
use crate::action::Ctx;

/// One skill's summary.
#[derive(Debug, Clone, Serialize)]
pub struct SkillSummary {
    /// The skill's id — its folder name under its root.
    pub id: RepoName,
    /// The skill's description, from its `SKILL.md` frontmatter.
    pub description: String,
    /// Which root defines it — the committed hall directory, or the personal
    /// one that never enters git.
    pub root: SkillRoot,
}

/// What `ivar skill list` found.
#[derive(Debug, Clone, Serialize)]
pub struct ListOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// One entry per skill, sorted by id.
    pub skills: Vec<SkillSummary>,
}

impl WriteHuman for ListOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.skills.is_empty() {
            writeln!(w, "No skills in {}.", self.root)?;
            return Ok(());
        }
        writeln!(w, "Skills in {}:", self.root)?;
        for skill in &self.skills {
            // Origin is printed for personal skills only. A hall skill is the
            // shared, expected case and needs no annotation; a personal one is
            // invisible to everyone else, which is worth saying out loud.
            let origin = match skill.root {
                SkillRoot::Hall => "",
                SkillRoot::Local => "  (local)",
            };
            writeln!(w, "  {}  {}{origin}", skill.id, skill.description)?;
        }
        Ok(())
    }
}
/// List the skills in both roots.
///
/// A skill whose `SKILL.md` is unreadable or has no parseable frontmatter is
/// listed with an empty description rather than failing the whole listing —
/// a status command does not hide seven skills because one is malformed.
/// (`doctor` is where the malformation gets named.)
///
/// This is why `list` scans directories itself instead of calling
/// [`enumerate`](super::enumerate::enumerate): the shared enumerator drops a
/// skill it cannot parse, which is right for `sync` (an unparseable skill
/// cannot be materialised) and wrong here (the user asked what is on disk).
/// A colliding id is therefore *shown twice*, once per root, rather than
/// hidden — `sync` is where the refusal belongs.
pub fn list(ctx: &Ctx) -> Outcome<ListOutcome> {
    let layout = discover_hall(ctx)?;
    let mut skills = Vec::new();

    for (dir, root) in [
        (layout.hall_skills(), SkillRoot::Hall),
        (layout.hall_skills_local(), SkillRoot::Local),
    ] {
        if !fs::is_dir(&dir)? {
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let Some(name) = entry.file_name() else {
                continue;
            };
            let Ok(id) = RepoName::new(name) else {
                continue;
            };
            skills.push(SkillSummary {
                description: read_description(&dir.join(name)),
                id,
                root,
            });
        }
    }

    skills.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(Report::new(ListOutcome {
        root: layout.root().to_path_buf(),
        skills,
    }))
}

/// The `description` from a skill's `SKILL.md` frontmatter, or an empty
/// string when it cannot be read.
fn read_description(skill_dir: &camino::Utf8Path) -> String {
    let Ok(Some(source)) = fs::read_text(&skill_dir.join("SKILL.md")) else {
        return String::new();
    };
    frontmatter::parse::<SkillMeta>(&source)
        .map(|meta| meta.description)
        .unwrap_or_default()
}

/// The frontmatter shape every `SKILL.md` carries.
#[derive(Debug, serde::Deserialize)]
struct SkillMeta {
    #[allow(dead_code)]
    name: String,
    description: String,
}

#[cfg(test)]
#[path = "../../../tests/unit/action/skill/list.rs"]
mod tests;
