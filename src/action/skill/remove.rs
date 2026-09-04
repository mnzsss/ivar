//! `ivar skill remove <skill>` — remove a skill.
//!
//! Removes the skill directory from `.ivar/skills/`, tears down all materialised
//! targets (via [`store::render::remove`]), and purges the lockfile entry so
//! state is fully cleaned beyond just files.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::{Ctx, Done};
use crate::domain::name::RepoName;
use crate::domain::skill::SkillRoot;
use crate::domain::skill_sync::TargetId;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;
use crate::store::render;
use crate::store::skill;

use super::super::discover_hall;

#[derive(Debug, Clone)]
pub struct RemoveInput {
    pub skill: String,
}

/// What `ivar skill remove` did.
#[derive(Debug, Clone, Serialize)]
pub struct RemoveOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The skill's id that was removed.
    pub id: RepoName,
    /// Number of target paths torn down.
    pub targets_removed: u64,
}

impl WriteHuman for RemoveOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Removed skill `{}`.", self.id)
    }
}

pub fn remove(ctx: &Ctx, input: RemoveInput) -> Outcome<Done> {
    let layout = discover_hall(ctx)?;

    // Find the skill in either root. No flag: an id names at most one
    // directory, because a collision is refused during enumeration.
    let Some((skill_dir, root)) = super::enumerate::resolve(&layout, &input.skill)? else {
        return Err(Failure::blocked(
            "skill.not_found",
            format!("skill `{}` does not exist", input.skill),
        )
        .expected("a skill directory in either skills root")
        .actual(format!(
            "no directory at `{}` or `{}`",
            layout.hall_skills_local().join(&input.skill),
            layout.hall_skills().join(&input.skill)
        ))
        .fix(FixAction::safe(
            "skill.list",
            "List available skills to find the correct id.",
        )));
    };

    // Parse the skill to know its id and source type.
    let skill = skill::parse_skill(skill_dir.clone(), root).map_err(|e| {
        Failure::failed(
            "skill.parse_error",
            format!("could not parse skill `{}`: {}", input.skill, e),
        )
        .fix(FixAction::safe(
            "skill.fix_frontmatter",
            "Fix the SKILL.md frontmatter, then try again.",
        ))
    })?;

    let skill = skill.ok_or_else(|| {
        Failure::blocked(
            "skill.no_frontmatter",
            format!("skill `{}` has no valid frontmatter", input.skill),
        )
        .expected("a SKILL.md with name in frontmatter")
        .actual("no parseable frontmatter found")
        .fix(FixAction::safe(
            "skill.add_frontmatter",
            "Add a `name` field to the SKILL.md frontmatter.",
        ))
    })?;

    // Tear down materialised targets best-effort.
    for target_id in TargetId::ALL {
        let target_path = skill::target_path(target_id, skill.id.as_str());
        if fs::exists(&target_path).unwrap_or(false) {
            let step = crate::domain::skill_sync::Step {
                skill: skill.id.clone(),
                target: target_path,
                source: skill.dir.clone(),
                action: crate::domain::skill_sync::Action::Remove,
                mode: skill.render_mode(),
                reason: None,
            };
            let _ = render::remove(&step);
        }
    }

    // Remove the skill directory itself.
    fs::remove_path(&skill_dir)?;

    // Purge the lockfile entry so state is fully cleaned — from the state
    // file of the root that owned the skill, never the other one.
    purge_lockfile_entry(layout.root(), root, &skill.id);

    Ok(Report::new(Done))
}

/// Remove a skill's entry from its own root's installation state file.
fn purge_lockfile_entry(hall_root: &camino::Utf8Path, root: SkillRoot, skill_id: &RepoName) {
    let Some(mut state) = skill::read(hall_root, root).ok().flatten() else {
        return;
    };
    state.installations.remove(skill_id.as_str());
    if state.installations.is_empty() {
        // Removing the last skill leaves no state to record — delete the
        // lockfile so `read` answers `None` (nothing installed), not an
        // empty-but-present file.
        let _ = crate::infra::fs::remove_path(&skill::state_path(hall_root, root));
    } else {
        let _ = skill::write(hall_root, root, &state);
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/skill/remove.rs"]
mod tests;
