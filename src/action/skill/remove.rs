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
    let skills_dir = layout.hall_skills();

    // Find the skill directory.
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

    // Parse the skill to know its id and source type.
    let skill = skill::parse_skill(skill_dir.clone()).map_err(|e| {
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
    for target_id in [TargetId::Claude, TargetId::OpenCode] {
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

    // Purge the lockfile entry so state is fully cleaned.
    purge_lockfile_entry(layout.root(), &skill.id);

    Ok(Report::new(Done))
}

/// Remove a skill's entry from the installation state file.
fn purge_lockfile_entry(hall_root: &camino::Utf8Path, skill_id: &RepoName) {
    let Some(mut state) = skill::read(hall_root).ok().flatten() else {
        return;
    };
    state.installations.remove(skill_id.as_str());
    if state.installations.is_empty() {
        // Removing the last skill leaves no state to record — delete the
        // lockfile so `read` answers `None` (nothing installed), not an
        // empty-but-present file.
        let path = hall_root.join(".ivar").join("skills").join("state.json");
        let _ = crate::infra::fs::remove_path(&path);
    } else {
        let _ = skill::write(hall_root, &state);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::action::hall::{self, InitInput};
    use crate::action::skill::sync;
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
    fn remove_deletes_the_skill_directory_and_targets() {
        let (_guard, root) = seeded_hall();
        write_skill(&root, "to_remove");
        let ctx = Ctx::new(root.clone());

        // First sync to create targets.
        let _ = sync::sync(&ctx).unwrap();

        assert!(fs::exists(&root.join(".claude").join("skills").join("to_remove")).unwrap());
        assert!(fs::exists(&root.join(".opencode").join("skills").join("to_remove")).unwrap());

        remove(
            &ctx,
            RemoveInput {
                skill: "to_remove".to_owned(),
            },
        )
        .unwrap();

        // Skill directory gone.
        assert!(!fs::exists(&root.join(".ivar").join("skills").join("to_remove")).unwrap());
        // Targets torn down.
        assert!(!fs::exists(&root.join(".claude").join("skills").join("to_remove")).unwrap());
        assert!(!fs::exists(&root.join(".opencode").join("skills").join("to_remove")).unwrap());
    }

    #[test]
    fn remove_purges_the_lockfile_entry() {
        let (_guard, root) = seeded_hall();
        write_skill(&root, "locked");
        let ctx = Ctx::new(root.clone());

        // Sync to create state.
        let _ = sync::sync(&ctx).unwrap();

        // Verify state exists.
        let state = skill::read(&root).unwrap().unwrap();
        assert_eq!(state.installations.len(), 1);

        remove(
            &ctx,
            RemoveInput {
                skill: "locked".to_owned(),
            },
        )
        .unwrap();

        // Lockfile entry purged.
        let state = skill::read(&root).unwrap();
        assert!(
            state.is_none(),
            "lockfile should be absent after removing last skill"
        );
    }

    #[test]
    fn remove_rejects_a_nonexistent_skill() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root);

        let failure = remove(
            &ctx,
            RemoveInput {
                skill: "ghost".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.code, "skill.not_found");
        assert_eq!(failure.status, crate::error::Status::Blocked);
    }

    #[test]
    fn remove_is_verifiable_by_state_cleanup() {
        let (_guard, root) = seeded_hall();
        write_skill(&root, "verify_me");
        let ctx = Ctx::new(root.clone());

        // Sync + remove.
        let _ = sync::sync(&ctx).unwrap();
        remove(
            &ctx,
            RemoveInput {
                skill: "verify_me".to_owned(),
            },
        )
        .unwrap();

        // No state entries remain.
        let state = skill::read(&root).unwrap();
        assert!(state.is_none());

        // No skill directory remains.
        let skills_dir = root.join(".ivar").join("skills");
        let entries = fs::read_dir(&skills_dir).unwrap();
        assert!(entries.is_empty());
    }
}
