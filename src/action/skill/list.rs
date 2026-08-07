//! `ivar skill list` — the skills in the hall's shared skills directory.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::error::{Outcome, Report, WriteHuman};
use crate::infra::fs;
use crate::infra::frontmatter;

use super::super::discover_hall;
use crate::action::Ctx;

/// One skill's summary.
#[derive(Debug, Clone, Serialize)]
pub struct SkillSummary {
    /// The skill's id — its folder name under `.ivar/skills/`.
    pub id: RepoName,
    /// The skill's description, from its `SKILL.md` frontmatter.
    pub description: String,
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
            writeln!(w, "  {}  {}", skill.id, skill.description)?;
        }
        Ok(())
    }
}

/// List the skills under `.ivar/skills/`.
///
/// A skill whose `SKILL.md` is unreadable or has no parseable frontmatter is
/// listed with an empty description rather than failing the whole listing —
/// a status command does not hide seven skills because one is malformed.
/// (`doctor` is where the malformation gets named.)
pub fn list(ctx: &Ctx) -> Outcome<ListOutcome> {
    let layout = discover_hall(ctx)?;

    let skills_dir = layout.hall_skills();
    let mut skills = Vec::new();
    if fs::is_dir(&skills_dir)? {
        for entry in fs::read_dir(&skills_dir)? {
            let Some(name) = entry.file_name() else {
                continue;
            };
            let Ok(id) = RepoName::new(name) else {
                continue;
            };
            skills.push(SkillSummary {
                description: read_description(&skills_dir.join(name)),
                id,
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
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::action::hall::{self, InitInput};
    use crate::store::layout::Layout;
    use crate::test_support::hall_root;

    fn seeded_hall() -> (tempfile::TempDir, Utf8PathBuf) {
        let (guard, root) = hall_root();
        let ctx = Ctx::new(root.clone());
        hall::init(
            &ctx,
            InitInput {
                path: Utf8PathBuf::from("."),
                name: Some("acme".to_owned()),
                provider: None,
            },
        )
        .unwrap();
        (guard, root)
    }

    fn write_skill(root: &camino::Utf8Path, id: &str, description: &str) {
        let dir = Layout::at(root.to_path_buf()).hall_skills().join(id);
        fs::ensure_dir(&dir).unwrap();
        fs::write_text(
            &dir.join("SKILL.md"),
            &format!("---\nname: {id}\ndescription: {description}\n---\n\nBody.\n"),
        )
        .unwrap();
    }

    #[test]
    fn list_reports_no_skills_in_a_fresh_hall() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root);

        let report = list(&ctx).unwrap();

        assert!(report.is_clean());
        assert!(report.value.skills.is_empty());
    }

    #[test]
    fn list_reports_skills_with_their_descriptions() {
        let (_guard, root) = seeded_hall();
        write_skill(&root, "refactor", "Safely restructure code");
        write_skill(&root, "audit", "Review a codebase for issues");
        let ctx = Ctx::new(root);

        let report = list(&ctx).unwrap();

        assert_eq!(report.value.skills.len(), 2);
        assert_eq!(report.value.skills[0].id.as_str(), "audit");
        assert_eq!(report.value.skills[0].description, "Review a codebase for issues");
        assert_eq!(report.value.skills[1].id.as_str(), "refactor");
    }

    #[test]
    fn a_malformed_skill_lists_with_an_empty_description() {
        let (_guard, root) = seeded_hall();
        let dir = Layout::at(root.clone()).hall_skills().join("broken");
        fs::ensure_dir(&dir).unwrap();
        fs::write_text(&dir.join("SKILL.md"), "no frontmatter here\n").unwrap();
        let ctx = Ctx::new(root);

        let report = list(&ctx).unwrap();

        assert_eq!(report.value.skills.len(), 1);
        assert_eq!(report.value.skills[0].id.as_str(), "broken");
        assert!(report.value.skills[0].description.is_empty());
    }

    #[test]
    fn the_human_surface_lists_skills_with_descriptions() {
        let outcome = ListOutcome {
            root: Utf8PathBuf::from("/hall"),
            skills: vec![SkillSummary {
                id: RepoName::new("audit").unwrap(),
                description: "Review a codebase".to_owned(),
            }],
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Skills in /hall:\n  audit  Review a codebase\n"
        );
    }
}
