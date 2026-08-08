//! `ivar skill add <repo> [--path] [--ref]` — install an external skill.
//!
//! Creates `<hall>/.ivar/skills/<id>/SKILL.md` with frontmatter recording the
//! upstream source (`repo`, `path`, `ref`). The skill's id is derived from the
//! last path segment of `repo` (e.g. `"owner/my-toolkit"` → `"my-toolkit"`).
//!
//! Refuses when a skill with that id already exists.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::domain::skill::ExternalRef;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;
use crate::action::Ctx;

/// What `ivar skill add` needs.
#[derive(Debug, Clone)]
pub struct AddInput {
    pub repo: String,
    pub path: Option<String>,
    pub ref_: Option<String>,
}

/// What `ivar skill add` did.
#[derive(Debug, Clone, Serialize)]
pub struct AddOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The skill's id — derived from the repo name.
    pub id: RepoName,
    /// The path of the written `SKILL.md`.
    pub skill_file: Utf8PathBuf,
}

impl WriteHuman for AddOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Added skill `{}` from {} at {}",
            self.id, self.skill_file, self.root
        )
    }
}

/// Install an external skill named after the repo's last path segment.
pub fn add(ctx: &Ctx, input: AddInput) -> Outcome<AddOutcome> {
    let layout = discover_hall(ctx)?;

    // Derive the skill id from the repo name — the last path segment.
    let id_str = input.repo.rsplit('/').next().ok_or_else(|| {
        Failure::failed("skill.add.bad_repo", "repo must contain at least one '/'")
    })?;

    let id = RepoName::new(id_str)?;

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

    let ext = ExternalRef {
        repo: input.repo.clone(),
        path: input.path.clone().unwrap_or_default(),
        git_ref: input.ref_.clone().unwrap_or_default(),
    };

    write_skill_markdown(&skill_file, &id, &ext)?;

    Ok(Report::new(AddOutcome {
        root: layout.root().to_path_buf(),
        id,
        skill_file,
    }))
}

/// Write the SKILL.md file with frontmatter recording the source.
fn write_skill_markdown(
    path: &camino::Utf8Path,
    id: &RepoName,
    ext: &ExternalRef,
) -> Result<(), Failure> {
    let lines: Vec<String> = vec![
        "---".to_owned(),
        format!("name: {id}"),
        format!("description: External skill from {}", ext.repo),
        "source:".to_owned(),
        format!("  repo: \"{}\"", ext.repo),
        format!("  path: \"{}\"", ext.path),
        format!("  ref: \"{}\"", ext.git_ref),
        "---".to_owned(),
        String::new(),
    ];
    let content = lines.join("\n");
    fs::write_text(path, &content).map_err(|e| e.into())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::action::hall::{self, InitInput};
    use crate::error::Status;
    use crate::store::layout::Layout;
    use crate::store::skill::parse_frontmatter;
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
    fn add_writes_sk_md_with_source_frontmatter() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());

        let report = add(
            &ctx,
            AddInput {
                repo: "mnzsss/skills".to_owned(),
                path: Some("skills/lint".to_owned()),
                ref_: Some("main".to_owned()),
            },
        )
        .unwrap();

        assert!(report.is_clean());
        assert_eq!(report.value.id.as_str(), "skills");

        let source = fs::read_text(&report.value.skill_file).unwrap().unwrap();
        let meta = parse_frontmatter(&source).unwrap().unwrap();

        assert_eq!(meta.name, "skills");
        assert_eq!(meta.source.as_ref().unwrap().repo, "mnzsss/skills");
        assert_eq!(meta.source.as_ref().unwrap().path, "skills/lint");
        assert_eq!(meta.source.as_ref().unwrap().git_ref, "main");
    }

    #[test]
    fn add_with_defaults_uses_empty_path_and_ref() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());

        let _report = add(
            &ctx,
            AddInput {
                repo: "owner/toolkit".to_owned(),
                path: None,
                ref_: None,
            },
        )
        .unwrap();

        let skill_dir = Layout::at(root).hall_skills().join("toolkit");
        let source = fs::read_text(&skill_dir.join("SKILL.md")).unwrap().unwrap();
        let meta = parse_frontmatter(&source).unwrap().unwrap();

        assert_eq!(meta.source.as_ref().unwrap().repo, "owner/toolkit");
        assert_eq!(meta.source.as_ref().unwrap().path, "");
        assert_eq!(meta.source.as_ref().unwrap().git_ref, "");
    }

    #[test]
    fn add_is_rejected_for_a_duplicate_skill() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());

        add(
            &ctx,
            AddInput {
                repo: "owner/toolkit".to_owned(),
                path: None,
                ref_: None,
            },
        )
        .unwrap();

        let failure = add(
            &ctx,
            AddInput {
                repo: "other/toolkit".to_owned(),
                path: None,
                ref_: None,
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "skill.already_exists");
    }

    #[test]
    fn add_derives_id_from_last_path_segment_of_repo() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());

        let report = add(
            &ctx,
            AddInput {
                repo: "deep/owner/repo-name".to_owned(),
                path: None,
                ref_: None,
            },
        )
        .unwrap();

        assert_eq!(report.value.id.as_str(), "repo-name");
    }
}
