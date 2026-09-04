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
use crate::error::{Outcome, Report, WriteHuman};
#[cfg(test)]
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

    // Both roots. A colliding id is dropped here for the same reason `sync`
    // drops it: neither copy can be materialised, so neither has a status.
    let (skills, _warnings) =
        super::enumerate::enumerate_both(&layout.hall_skills(), &layout.hall_skills_local())?;

    // Build status entries.
    let mut skill_statuses = Vec::new();
    for skill in &skills {
        let source_label = match &skill.source {
            crate::domain::skill::Source::Authored => "authored".to_owned(),
            crate::domain::skill::Source::External(_) => "external".to_owned(),
        };

        let mut targets = Vec::new();
        for target_id in crate::domain::skill_sync::TargetId::ALL {
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

#[cfg(test)]
#[path = "../../../tests/unit/action/skill/status.rs"]
mod tests;
