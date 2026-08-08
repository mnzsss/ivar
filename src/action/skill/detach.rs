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

    fn write_external_skill(root: &camino::Utf8Path, id: &str) {
        let dir = Layout::at(root.to_path_buf()).hall_skills().join(id);
        fs::ensure_dir(&dir).unwrap();
        let content = "---\nname: ext-skill\ndescription: An external skill\nsource:\n  repo: owner/repo\n  path: skills/ext\n  ref: main\n---\n\nThis is the body.\nIt should stay.\n";
        fs::write_text(&dir.join("SKILL.md"), content).unwrap();
    }

    fn write_authored_skill(root: &camino::Utf8Path, id: &str) {
        let dir = Layout::at(root.to_path_buf()).hall_skills().join(id);
        fs::ensure_dir(&dir).unwrap();
        let content = "---\nname: auth-skill\ndescription: An authored skill\n---\n\nBody only.\n";
        fs::write_text(&dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn detach_removes_source_from_frontmatter_and_preserves_body() {
        let (_guard, root) = seeded_hall();
        write_external_skill(&root, "detach_me");
        let ctx = Ctx::new(root.clone());

        detach(
            &ctx,
            DetachInput {
                skill: "detach_me".to_owned(),
            },
        )
        .unwrap();

        // Verify the source field is gone.
        let raw = fs::read_text(
            &root
                .join(".ivar")
                .join("skills")
                .join("detach_me")
                .join("SKILL.md"),
        )
        .unwrap()
        .unwrap();

        // Parse frontmatter and check no source.
        let fm = skill::parse_frontmatter(&raw).unwrap().unwrap();
        assert!(fm.source.is_none(), "source should be removed after detach");
        assert_eq!(fm.name, "ext-skill");
        assert_eq!(fm.description, Some("An external skill".to_owned()));
    }

    #[test]
    fn detach_of_authored_is_a_no_op() {
        let (_guard, root) = seeded_hall();
        write_authored_skill(&root, "authored");
        let ctx = Ctx::new(root.clone());

        let result = detach(
            &ctx,
            DetachInput {
                skill: "authored".to_owned(),
            },
        );

        // Returns success (no error) — it's a no-op.
        result.unwrap();

        // Frontmatter unchanged (still no source).
        let raw = fs::read_text(
            &root
                .join(".ivar")
                .join("skills")
                .join("authored")
                .join("SKILL.md"),
        )
        .unwrap()
        .unwrap();
        let fm = skill::parse_frontmatter(&raw).unwrap().unwrap();
        assert!(fm.source.is_none());
    }

    #[test]
    fn detach_rejects_a_nonexistent_skill() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root);

        let failure = detach(
            &ctx,
            DetachInput {
                skill: "ghost".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.code, "skill.not_found");
    }

    #[test]
    fn detach_preserves_body_text_unchanged() {
        let (_guard, root) = seeded_hall();
        write_external_skill(&root, "body_test");
        let ctx = Ctx::new(root.clone());

        detach(
            &ctx,
            DetachInput {
                skill: "body_test".to_owned(),
            },
        )
        .unwrap();

        let raw = fs::read_text(
            &root
                .join(".ivar")
                .join("skills")
                .join("body_test")
                .join("SKILL.md"),
        )
        .unwrap()
        .unwrap();
        // Body should contain the original text.
        assert!(raw.contains("This is the body."));
        assert!(raw.contains("It should stay."));
    }
}
