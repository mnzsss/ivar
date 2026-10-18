//! `ivar skill enable` / `ivar skill disable` — turn a skill on or off
//! without deleting it.
//!
//! The choice is recorded in the personal `.ivar/skills-local/disabled.json`
//! and applied through the same reconciliation `ivar skill sync` runs, so a
//! disabled skill loses its provider targets and stays out of every later
//! sync until it is enabled again.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::name::RepoName;
use crate::domain::skill::SkillRoot;
use crate::error::{Outcome, Report, WriteHuman};
use crate::store::skill;

use super::super::{discover_hall, read_manifest};

/// What `ivar skill enable` and `ivar skill disable` need.
#[derive(Debug, Clone)]
pub struct ToggleInput {
    /// The skill's id, from either skills root.
    pub skill: String,
}

/// What `ivar skill enable` or `ivar skill disable` did.
#[derive(Debug, Clone, Serialize)]
pub struct ToggleOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The skill's id.
    pub id: RepoName,
    /// The skills root that owns the skill.
    pub origin: SkillRoot,
    /// Whether the skill is enabled after the command.
    pub enabled: bool,
    /// Whether the command changed the skill's state.
    pub changed: bool,
    /// Number of skill targets changed during reconciliation.
    pub skill_targets: u64,
}

impl WriteHuman for ToggleOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let state = if self.enabled { "enabled" } else { "disabled" };
        if !self.changed {
            return writeln!(w, "Skill `{}` is already {state}.", self.id);
        }
        let verb = if self.enabled { "Enabled" } else { "Disabled" };
        let origin = match self.origin {
            SkillRoot::Hall => "hall",
            SkillRoot::Local => "local",
        };
        writeln!(
            w,
            "{verb} skill `{}` ({origin}). Targets changed: {}.",
            self.id, self.skill_targets
        )
    }
}

/// Turn a skill back on and materialise it for the hall's providers.
pub fn enable(ctx: &Ctx, input: &ToggleInput) -> Outcome<ToggleOutcome> {
    set_enabled(ctx, input, true)
}

/// Turn a skill off and remove its provider targets, keeping its source.
pub fn disable(ctx: &Ctx, input: &ToggleInput) -> Outcome<ToggleOutcome> {
    set_enabled(ctx, input, false)
}

fn set_enabled(ctx: &Ctx, input: &ToggleInput, enabled: bool) -> Outcome<ToggleOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let id =
        RepoName::new(&input.skill).map_err(|_| super::skill_not_found(&layout, &input.skill))?;
    let Some((_dir, origin)) = super::enumerate::resolve(&layout, id.as_str())? else {
        return Err(super::skill_not_found(&layout, id.as_str()));
    };
    let mut disabled = skill::read_disabled(layout.root())?;
    let changed = if enabled {
        disabled.disabled.remove(id.as_str())
    } else {
        disabled.disabled.insert(id.as_str().to_owned())
    };
    let mut warnings = Vec::new();
    let skill_targets = if changed {
        skill::write_disabled(layout.root(), &disabled)?;
        super::sync::reconcile(&layout, &manifest, &mut warnings)?
    } else {
        0
    };
    Ok(Report::with_warnings(
        ToggleOutcome {
            root: layout.root().to_path_buf(),
            id,
            origin,
            enabled,
            changed,
            skill_targets,
        },
        warnings,
    ))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/skill/toggle.rs"]
mod tests;
