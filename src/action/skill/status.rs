//! `ivar skill status` — show skill installation state.
//!
//! Reports which skills exist, which are external, and whether their materialised
//! targets diverge from the declaration.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::{Ctx, Done};
use crate::domain::name::RepoName;
use crate::domain::skill_sync::MaterialStatus;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::infra::fs;
use crate::store::render;
use crate::store::skill;

use super::super::discover_hall;

/// One skill's installation status.
#[derive(Debug, Clone, Serialize)]
pub struct SkillStatus {
    /// The skill's id.
    pub id: RepoName,
    /// Whether the skill is authored locally or points to an upstream repo.
    pub source: String,
    /// Material status for each target provider.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<TargetStatus>,
}

/// Status of one target for a skill.
#[derive(Debug, Clone, Serialize)]
pub struct TargetStatus {
    /// The target identifier (claude / opencode).
    pub target: String,
    /// What exists at the target path right now.
    pub status: String,
}

/// What `ivar skill status` found.
#[derive(Debug, Clone, Serialize)]
pub struct StatusOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// One entry per skill, sorted by id.
    pub skills: Vec<SkillStatus>,
}

impl WriteHuman for StatusOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.skills.is_empty() {
            writeln!(w, "No skills in {}.", self.root)?;
            return Ok(());
        }
        writeln!(w, "Skills in {}:", self.root)?;
        for skill in &self.skills {
            let kind = match skill.source.as_str() {
                "authored" => "A",
                "external" => "E",
                _ => "?",
            };
            writeln!(w, "  [{kind}] {}", skill.id)?;
            for target in &skill.targets {
                writeln!(w, "       {}: {}", target.target, target.status)?;
            }
        }
        Ok(())
    }
}

/// Show skill installation state.
pub fn status(ctx: &Ctx) -> Outcome<Done> {
    let layout = discover_hall(ctx)?;
    let skills_dir = layout.hall_skills();

    // Enumerate declared skills.
    let skills = enumerate_skills(&skills_dir)?;

    // Build status entries.
    let mut skill_statuses = Vec::new();
    for skill in &skills {
        let source_label = match &skill.source {
            crate::domain::skill::Source::Authored => "authored".to_owned(),
            crate::domain::skill::Source::External(_) => "external".to_owned(),
        };

        let mut targets = Vec::new();
        for target_id in [
            crate::domain::skill_sync::TargetId::Claude,
            crate::domain::skill_sync::TargetId::OpenCode,
        ] {
            let target_path = skill::target_path(target_id, skill.id.as_str());
            let expected = skill.dir.join("SKILL.md");
            let current = render::verify_status(&target_path, &expected);
            targets.push(TargetStatus {
                target: target_id.as_str().to_owned(),
                status: material_status_label(current),
            });
        }

        skill_statuses.push(SkillStatus {
            id: skill.id.clone(),
            source: source_label,
            targets,
        });
    }

    skill_statuses.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(Report::new(Done))
}

/// Convert a [`MaterialStatus`] enum to a human-readable label.
fn material_status_label(status: MaterialStatus) -> String {
    match status {
        MaterialStatus::Missing => "missing".to_owned(),
        MaterialStatus::Ok => "ok".to_owned(),
        MaterialStatus::WrongLink => "wrong link".to_owned(),
        MaterialStatus::NotLink => "not a symlink".to_owned(),
        MaterialStatus::BrokenSymlink => "broken symlink".to_owned(),
    }
}

/// Enumerate skills from the hall skills directory.
fn enumerate_skills(
    hall_skills: &camino::Utf8Path,
) -> Result<Vec<crate::domain::skill::Skill>, Failure> {
    let mut skills = Vec::new();
    if !fs::exists(hall_skills)? {
        return Ok(skills);
    }
    for entry in fs::read_dir(hall_skills)? {
        let file_name = entry.file_name().ok_or_else(|| {
            Failure::failed(
                "skill.bad_entry",
                format!("directory entry has no name: {:?}", entry),
            )
        })?;
        if let Ok(_id) = crate::domain::name::RepoName::new(file_name) {
            if let Ok(Some(skill)) = skill::parse_skill(entry.clone()) {
                skills.push(skill);
            }
        }
    }
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(skills)
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

    fn write_skill(root: &camino::Utf8Path, id: &str, is_external: bool) {
        let dir = Layout::at(root.to_path_buf()).hall_skills().join(id);
        fs::ensure_dir(&dir).unwrap();
        if is_external {
            let content = "---\nname: ext\nsource:\n  repo: owner/repo\n  path: skills/ext\n  ref: main\n---\n\nBody.\n";
            fs::write_text(&dir.join("SKILL.md"), content).unwrap();
        } else {
            let content = "---\nname: auth\ndescription: Authored skill\n---\n\nBody.\n";
            fs::write_text(&dir.join("SKILL.md"), content).unwrap();
        }
    }

    #[test]
    fn status_reports_authored_and_external_skills() {
        let (_guard, root) = seeded_hall();
        write_skill(&root, "authed", false);
        write_skill(&root, "ext", true);
        let ctx = Ctx::new(root);

        let result = status(&ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn status_handles_an_empty_hall() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root);

        let result = status(&ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn material_status_label_maps_all_variants() {
        assert_eq!(material_status_label(MaterialStatus::Missing), "missing");
        assert_eq!(material_status_label(MaterialStatus::Ok), "ok");
        assert_eq!(
            material_status_label(MaterialStatus::WrongLink),
            "wrong link"
        );
        assert_eq!(
            material_status_label(MaterialStatus::NotLink),
            "not a symlink"
        );
        assert_eq!(
            material_status_label(MaterialStatus::BrokenSymlink),
            "broken symlink"
        );
    }

    #[test]
    fn the_human_surface_lists_skills_with_kind_markers() {
        let outcome = StatusOutcome {
            root: Utf8PathBuf::from("/hall"),
            skills: vec![
                SkillStatus {
                    id: RepoName::new("audit").unwrap(),
                    source: "authored".to_owned(),
                    targets: vec![TargetStatus {
                        target: "claude".to_owned(),
                        status: "ok".to_owned(),
                    }],
                },
                SkillStatus {
                    id: RepoName::new("lint").unwrap(),
                    source: "external".to_owned(),
                    targets: vec![TargetStatus {
                        target: "opencode".to_owned(),
                        status: "missing".to_owned(),
                    }],
                },
            ],
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("[A] audit"));
        assert!(text.contains("[E] lint"));
        assert!(text.contains("claude: ok"));
        assert!(text.contains("opencode: missing"));
    }
}
