//! `ivar skill create <id> --description "..."` — scaffold a new skill.
//!
//! Creates `<hall>/.ivar/skills/<id>/SKILL.md` with `name` and `description`
//! frontmatter. Refuses when the skill already exists — a skill is a folder
//! of files a team may have grown; regenerating it would discard that work.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;
use crate::action::Ctx;

/// What `ivar skill create` needs.
#[derive(Debug, Clone)]
pub struct CreateInput {
    /// The skill's id — one path segment, unique within the skills dir.
    pub id: String,
    /// The skill's description, for the `SKILL.md` frontmatter.
    pub description: String,
}

/// What `ivar skill create` did.
#[derive(Debug, Clone, Serialize)]
pub struct CreateOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The skill's id.
    pub id: RepoName,
    /// The path of the created `SKILL.md`.
    pub skill_file: Utf8PathBuf,
}

impl WriteHuman for CreateOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Created skill `{}` at {}", self.id, self.skill_file)
    }
}

/// Scaffold a skill named `input.id` with `input.description`.
///
/// Blocked when a skill with that id already exists, and when the id is not
/// a valid repo-name segment (the same rules apply: it becomes a directory).
pub fn create(ctx: &Ctx, input: CreateInput) -> Outcome<CreateOutcome> {
    let layout = discover_hall(ctx)?;
    let id = RepoName::new(input.id)?;

    let skill_dir = layout.hall_skills().join(id.as_str());
    let skill_file = skill_dir.join("SKILL.md");
    if fs::exists(&skill_file)? {
        return Err(Failure::blocked(
            "skill.already_exists",
            format!("skill `{id}` already exists"),
        )
        .expected("a skill id that has not been used before")
        .actual(format!("`{skill_file}` already exists"))
        .fix(FixAction::safe(
            "skill.use_existing",
            "Edit the existing skill, or remove it deliberately first.",
        )));
    }

    fs::ensure_dir(&skill_dir)?;
    fs::write_text(
        &skill_file,
        &format!(
            "---\nname: {id}\ndescription: {}\n---\n\n",
            input.description
        ),
    )?;

    Ok(Report::new(CreateOutcome {
        root: layout.root().to_path_buf(),
        id,
        skill_file,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::action::hall::{self, InitInput};
    use crate::error::Status;
    use crate::infra::frontmatter;
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

    #[test]
    fn create_writes_sk_md_with_name_and_description_frontmatter() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());

        let report = create(
            &ctx,
            CreateInput {
                id: "refactor".to_owned(),
                description: "Safely restructure code".to_owned(),
            },
        )
        .unwrap();

        assert!(report.is_clean());
        let source = fs::read_text(&report.value.skill_file).unwrap().unwrap();
        let meta: serde_json::Value = frontmatter::parse::<serde_json::Value>(&source).unwrap();
        assert_eq!(meta.get("name").unwrap(), "refactor");
        assert_eq!(meta.get("description").unwrap(), "Safely restructure code");
    }

    #[test]
    fn create_is_rejected_for_a_duplicate_skill() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        create(
            &ctx,
            CreateInput {
                id: "refactor".to_owned(),
                description: "first".to_owned(),
            },
        )
        .unwrap();

        let failure = create(
            &ctx,
            CreateInput {
                id: "refactor".to_owned(),
                description: "second".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "skill.already_exists");
    }

    #[test]
    fn create_rejects_an_invalid_id() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root);

        let failure = create(
            &ctx,
            CreateInput {
                id: "../etc".to_owned(),
                description: "x".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "name.not_a_segment");
    }

    #[test]
    fn the_human_surface_names_the_skill_file() {
        let outcome = CreateOutcome {
            root: Utf8PathBuf::from("/hall"),
            id: RepoName::new("refactor").unwrap(),
            skill_file: Utf8PathBuf::from("/hall/.ivar/skills/refactor/SKILL.md"),
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Created skill `refactor` at /hall/.ivar/skills/refactor/SKILL.md\n"
        );
    }
}
