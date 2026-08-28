//! `ivar skill doctor` — health diagnostics with fix_actions.
//!
//! Scans all declared skills and their materialised targets, reports every
//! problem found, and provides a named fix action for each one.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::skill_sync::{MaterialStatus, Target, TargetId};
use crate::error::{FixAction, Outcome, Report, WriteHuman};
#[cfg(test)]
use crate::infra::fs;
use crate::store::render;
use crate::store::skill;

use super::super::discover_hall;

/// One problem found by the doctor.
#[derive(Debug, Clone, Serialize)]
pub struct Problem {
    /// Stable identifier for this kind of problem.
    pub code: &'static str,
    /// What the problem is about (skill id).
    pub subject: String,
    /// Human-readable description.
    pub what: String,
    /// A named fix the user can run.
    pub fix_action: FixAction,
}

/// What `ivar skill doctor` found.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// Number of problems found.
    pub count: u64,
    /// Every problem with its fix action.
    pub problems: Vec<Problem>,
}

impl WriteHuman for DoctorOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.problems.is_empty() {
            writeln!(w, "No problems found.")?;
            return Ok(());
        }
        writeln!(
            w,
            "{} problem{} found:",
            self.problems.len(),
            if self.problems.len() == 1 { "" } else { "s" }
        )?;
        for (i, problem) in self.problems.iter().enumerate() {
            writeln!(w, "  {}. {} ({})", i + 1, problem.what, problem.code)?;
            writeln!(w, "     fix: {}", problem.fix_action.what)?;
            if let Some(cmd) = &problem.fix_action.command {
                writeln!(w, "       $ {}", cmd)?;
            }
        }
        Ok(())
    }
}

/// Health diagnostics with fix actions.
pub fn doctor(ctx: &Ctx) -> Outcome<DoctorOutcome> {
    let layout = discover_hall(ctx)?;

    // Both roots. A collision is a health problem by definition — the id is
    // declared twice and materialises nowhere — so it is reported rather than
    // silently skipped, which is what `doctor` exists for.
    let (skills, collisions) =
        super::enumerate::enumerate_both(&layout.hall_skills(), &layout.hall_skills_local())?;

    // Read each root's state and merge; the diagnostics below reconcile one
    // view of what is declared against one view of what is recorded.
    let mut state = read_state(&layout, crate::domain::skill::SkillRoot::Hall);
    state
        .installations
        .extend(read_state(&layout, crate::domain::skill::SkillRoot::Local).installations);

    // Build targets for both providers.
    let mut targets = Vec::new();
    for skill in &skills {
        for target_id in [TargetId::Claude, TargetId::OpenCode] {
            // `target_path` is hall-relative; join onto the hall root and
            // verify the whole-directory link the renderer actually creates.
            let path = layout
                .root()
                .join(skill::target_path(target_id, skill.id.as_str()));
            let status = render::verify_status(&path, &skill.dir);
            targets.push(Target {
                id: target_id,
                skill: skill.id.clone(),
                path,
                source_path: skill.dir.clone(),
                source_hash: String::new(), // Not needed for diagnosis.
                status,
            });
        }
    }

    // Find problems.
    let mut problems = Vec::new();

    // An id declared in both roots materialises nowhere. Report it first: it
    // explains why a skill the user can see on disk is absent everywhere else.
    for collision in collisions {
        problems.push(Problem {
            code: "skill.collision",
            subject: collision.subject.clone(),
            what: collision.what.clone(),
            fix_action: FixAction::unsafe_(
                "skill.rename_one",
                "Rename one of the two directories so the id is unique.",
            ),
        });
    }

    for skill in &skills {
        // Check each target.
        for target_id in [TargetId::Claude, TargetId::OpenCode] {
            let target = targets
                .iter()
                .find(|t| t.skill == skill.id && t.id == target_id);
            if let Some(t) = target {
                match &t.status {
                    MaterialStatus::Missing => {
                        problems.push(Problem {
                            code: "skill.target_missing",
                            subject: format!("{}@{}", skill.id, target_id.as_str()),
                            what: format!(
                                "materialised target for `{}` at `{}` is missing",
                                skill.id, t.path
                            ),
                            fix_action: FixAction::safe(
                                "skill.sync",
                                "Run `ivar skill sync` to repair.",
                            )
                            .command("ivar skill sync"),
                        });
                    }
                    MaterialStatus::WrongLink => {
                        problems.push(Problem {
                            code: "skill.wrong_link",
                            subject: format!("{}@{}", skill.id, target_id.as_str()),
                            what: format!("symlink for `{}` points to the wrong target", skill.id),
                            fix_action: FixAction::safe(
                                "skill.sync",
                                "Run `ivar skill sync` to update the link.",
                            )
                            .command("ivy skill sync"),
                        });
                    }
                    MaterialStatus::NotLink => {
                        problems.push(Problem {
                            code: "skill.not_a_symlink",
                            subject: format!("{}@{}", skill.id, target_id.as_str()),
                            what: format!("target for `{}` exists but is not a symlink", skill.id),
                            fix_action: FixAction::safe(
                                "skill.sync",
                                "Run `ivar skill sync` to replace with a symlink.",
                            )
                            .command("ivar skill sync"),
                        });
                    }
                    MaterialStatus::BrokenSymlink => {
                        problems.push(Problem {
                            code: "skill.broken_symlink",
                            subject: format!("{}@{}", skill.id, target_id.as_str()),
                            what: format!(
                                "symlink for `{}` points to a non-existent target",
                                skill.id
                            ),
                            fix_action: FixAction::safe(
                                "skill.sync",
                                "Run `ivar skill sync` to repair the broken link.",
                            )
                            .command("ivar skill sync"),
                        });
                    }
                    MaterialStatus::Ok => {}
                }
            }
        }
    }

    // Check for untracked state entries (state references skills no longer declared).
    let declared: std::collections::HashSet<&str> = skills.iter().map(|s| s.id.as_str()).collect();
    for (id, entry) in &state.installations {
        if !declared.contains(id.as_str()) {
            for _provider in entry.providers.values() {
                problems.push(Problem {
                    code: "skill.orphaned_state",
                    subject: id.clone(),
                    what: format!(
                        "installation state references skill `{id}` which no longer exists",
                    ),
                    fix_action: FixAction::safe(
                        "skill.sync",
                        "Run `ivar skill sync` to clean up orphaned state.",
                    )
                    .command("ivar skill sync"),
                });
            }
        }
    }

    let count = problems.len() as u64;

    Ok(Report::new(DoctorOutcome {
        root: layout.root().to_path_buf(),
        count,
        problems,
    }))
}

/// Read one root's recorded state, treating an unreadable file as empty.
fn read_state(
    layout: &crate::store::layout::Layout,
    root: crate::domain::skill::SkillRoot,
) -> crate::domain::skill_sync::State {
    skill::read(layout.root(), root)
        .unwrap_or_default()
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "../../../tests/unit/action/skill/doctor.rs"]
mod tests;
