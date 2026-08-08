//! `ivar skill doctor` — health diagnostics with fix_actions.
//!
//! Scans all declared skills and their materialised targets, reports every
//! problem found, and provides a named fix action for each one.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::skill_sync::{MaterialStatus, Target, TargetId};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
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
    let skills_dir = layout.hall_skills();

    // Enumerate declared skills.
    let skills = enumerate_skills(&skills_dir)?;

    // Read current installation state.
    let state = skill::read(layout.root())
        .unwrap_or_default()
        .unwrap_or_default();

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
            for (_target_id, _provider) in &entry.providers {
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
    use crate::action::skill::sync as skill_sync;
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

    fn write_skill(root: &camino::Utf8Path, id: &str) {
        let dir = Layout::at(root.to_path_buf()).hall_skills().join(id);
        fs::ensure_dir(&dir).unwrap();
        fs::write_text(
            &dir.join("SKILL.md"),
            &format!("---\nname: {id}\ndescription: {id} skill\n---\n\nBody.\n"),
        )
        .unwrap();
    }

    #[test]
    fn doctor_reports_no_problems_in_a_fresh_synced_hall() {
        let (_guard, root) = seeded_hall();
        write_skill(&root, "healthy");
        let ctx = Ctx::new(root.clone());

        // Sync first to create targets.
        let _ = skill_sync::sync(&ctx).unwrap();

        let outcome = doctor(&ctx).unwrap();
        assert_eq!(outcome.value.count, 0);
        assert!(outcome.value.problems.is_empty());
    }

    #[test]
    fn doctor_detects_a_missing_target() {
        let (_guard, root) = seeded_hall();
        write_skill(&root, "broken");
        let ctx = Ctx::new(root.clone());

        // Sync once to create targets.
        let _ = skill_sync::sync(&ctx).unwrap();

        // Remove the Claude target.
        let claude_target = root.join(".claude").join("skills").join("broken");
        fs::remove_path(&claude_target).unwrap();

        let outcome = doctor(&ctx).unwrap();
        assert!(outcome.value.count > 0);
        assert!(
            outcome
                .value
                .problems
                .iter()
                .any(|p| p.code == "skill.target_missing")
        );
    }

    #[test]
    fn doctor_handles_an_empty_hall() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root);

        let outcome = doctor(&ctx).unwrap();
        assert_eq!(outcome.value.count, 0);
    }

    #[test]
    fn the_human_surface_reports_problems_with_fixes() {
        let outcome = DoctorOutcome {
            root: Utf8PathBuf::from("/hall"),
            count: 1,
            problems: vec![Problem {
                code: "skill.target_missing",
                subject: "audit@claude".to_owned(),
                what: "materialised target for `audit` at `/target` is missing".to_owned(),
                fix_action: FixAction::safe("skill.sync", "Run `ivy skill sync` to repair.")
                    .command("ivy skill sync"),
            }],
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("1 problem found:"));
        assert!(text.contains("skill.target_missing"));
        assert!(text.contains("fix: Run `ivy skill sync` to repair."));
    }

    #[test]
    fn the_human_surface_reports_clean_state() {
        let outcome = DoctorOutcome {
            root: Utf8PathBuf::from("/hall"),
            count: 0,
            problems: Vec::new(),
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(String::from_utf8(out).unwrap(), "No problems found.\n");
    }
}
