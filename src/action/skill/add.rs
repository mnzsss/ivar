//! `ivar skill add <source> [--path] [--ref] [--hall]` — install external skills.
//!
//! Downloads the repository tarball from GitHub, scans for candidate skills
//! (folders directly containing a `SKILL.md`), filters by `--path` if specified,
//! prompts for multi-selection if multiple skills are found, and copies the chosen
//! skill folder(s) into `.ivar/skills-local/<id>/` (or `.ivar/skills/<id>/` with `--hall`).
//!
//! Refuses when a skill with the same id already exists in either root, or when
//! multiple candidates in the same repo share an id.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::action::Ctx;
use crate::action::confirm::SelectOption;
use crate::domain::name::RepoName;
use crate::domain::skill::ExternalRef;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::frontmatter;
use crate::infra::fs;

use super::super::discover_hall;
use super::source::parse_source;

/// What `ivar skill add` needs.
#[derive(Debug, Clone)]
pub struct AddInput {
    pub repo: String,
    pub path: Option<String>,
    pub ref_: Option<String>,
    /// Install into the committed hall root instead of the personal one.
    pub hall: bool,
}

/// Information about an installed skill.
#[derive(Debug, Clone, Serialize)]
pub struct InstalledSkill {
    pub id: RepoName,
    pub skill_file: Utf8PathBuf,
}

/// What `ivar skill add` did.
#[derive(Debug, Clone, Serialize)]
pub struct AddOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// Installed skills.
    pub skills: Vec<InstalledSkill>,
}

impl WriteHuman for AddOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.skills.len() == 1 {
            if let Some(first) = self.skills.first() {
                writeln!(
                    w,
                    "Added skill `{}` from {} at {}",
                    first.id, first.skill_file, self.root
                )?;
            }
            Ok(())
        } else {
            writeln!(w, "Added {} skill(s) to {}:", self.skills.len(), self.root)?;
            for skill in &self.skills {
                writeln!(w, "  - `{}` from {}", skill.id, skill.skill_file)?;
            }
            Ok(())
        }
    }
}

/// A candidate skill found inside an extracted repo tarball.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CandidateSkill {
    pub id: String,
    pub path: String,
    pub description: Option<String>,
    pub dir: Utf8PathBuf,
}

/// Discover all `SKILL.md` candidate skills inside an extracted repository.
pub(super) fn discover_candidates(temp_dir: &Utf8Path) -> Result<Vec<CandidateSkill>, Failure> {
    let repo_root = find_repo_root(temp_dir);
    let mut candidates = Vec::new();

    for entry in walkdir::WalkDir::new(repo_root.as_std_path()) {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                return Err(Failure::failed(
                    "skill.add.scan_error",
                    format!("error scanning extracted repository: {err}"),
                ));
            }
        };

        if entry.file_type().is_file() && entry.file_name() == "SKILL.md" {
            let file_path = Utf8Path::from_path(entry.path()).ok_or_else(|| {
                Failure::failed("skill.add.path_utf8", "path is not valid UTF-8".to_owned())
            })?;

            let skill_dir = file_path.parent().ok_or_else(|| {
                Failure::failed(
                    "skill.add.invalid_skill_file",
                    "SKILL.md has no parent directory".to_owned(),
                )
            })?;

            let id = skill_dir.file_name().unwrap_or("skill").to_owned();

            let rel_path = if skill_dir == repo_root {
                String::new()
            } else {
                let rel = skill_dir.strip_prefix(&repo_root).map_err(|_| {
                    Failure::failed(
                        "skill.add.strip_prefix",
                        "could not resolve relative skill path".to_owned(),
                    )
                })?;
                rel.as_str().replace('\\', "/")
            };

            let description = if let Ok(Some(text)) = fs::read_text(file_path) {
                crate::store::skill::parse_frontmatter(&text)
                    .ok()
                    .flatten()
                    .and_then(|fm| fm.description)
            } else {
                None
            };

            candidates.push(CandidateSkill {
                id,
                path: rel_path,
                description,
                dir: skill_dir.to_path_buf(),
            });
        }
    }

    Ok(candidates)
}

/// Find the single top-level directory in `temp_dir` if present; otherwise `temp_dir`.
fn find_repo_root(temp_dir: &Utf8Path) -> Utf8PathBuf {
    if let Ok(entries) = fs::read_dir(temp_dir) {
        let dirs: Vec<_> = entries
            .into_iter()
            .filter(|p| fs::is_dir(p).unwrap_or(false))
            .collect();
        if dirs.len() == 1 {
            return dirs
                .first()
                .cloned()
                .unwrap_or_else(|| temp_dir.to_path_buf());
        }
    }
    temp_dir.to_path_buf()
}

/// Install external skill(s) from a GitHub repository or subpath URL.
pub fn add(ctx: &Ctx, input: AddInput) -> Outcome<AddOutcome> {
    let layout = discover_hall(ctx)?;

    // 1. Parse source argument + flags into ExternalRef
    let ext = parse_source(&input.repo, input.path.as_deref(), input.ref_.as_deref())?;

    // 2. Fetch tarball from GitHub
    let tarball_bytes = crate::infra::github::fetch_tarball(&ext.repo, &ext.git_ref)?;

    // 3. Extract to temporary directory
    let temp_dir = fs::TempDir::new().map_err(|e| {
        Failure::failed(
            "skill.add.temp_dir",
            format!("could not create temporary directory: {e}"),
        )
    })?;

    super::extract_tarball_into(&tarball_bytes, temp_dir.path()).map_err(|e| {
        Failure::failed(
            "skill.add.extract_failed",
            format!("could not extract tarball: {e}"),
        )
    })?;

    // 4. Discover candidate skills
    let candidates = discover_candidates(temp_dir.path())?;

    // 5. Filter candidates by ext.path if specified
    let filtered: Vec<CandidateSkill> = if !ext.path.is_empty() {
        candidates
            .into_iter()
            .filter(|c| c.path == ext.path)
            .collect()
    } else {
        candidates
    };

    if filtered.is_empty() {
        if !ext.path.is_empty() {
            return Err(Failure::blocked(
                "skill.add.path_not_found",
                format!(
                    "no skill found at path `{}` in repository `{}`",
                    ext.path, ext.repo
                ),
            )
            .expected(format!(
                "a SKILL.md under path `{}` in `{}`",
                ext.path, ext.repo
            ))
            .actual("no SKILL.md found at that path")
            .fix(FixAction::safe(
                "skill.add.list_skills",
                "Run `ivar skill add <repo>` to see available skills in the repository.",
            )));
        }
        return Err(Failure::blocked(
            "skill.add.no_skills",
            format!("no skills found in repository `{}`", ext.repo),
        )
        .expected(format!(
            "at least one directory containing a SKILL.md in `{}`",
            ext.repo
        ))
        .actual("no SKILL.md found in repository")
        .fix(FixAction::safe(
            "skill.add.check_repo",
            "Verify that the repository contains a SKILL.md file.",
        )));
    }

    // Check for in-candidate duplicate IDs
    for (i, c1) in filtered.iter().enumerate() {
        for c2 in filtered.iter().skip(i + 1) {
            if c1.id == c2.id {
                return Err(Failure::blocked(
                    "skill.add.duplicate_id",
                    format!(
                        "multiple skills in repository `{}` share the same id `{}` (`{}` and `{}`); select one directly with --path",
                        ext.repo, c1.id, c1.path, c2.path
                    ),
                )
                .expected("unique skill ids across candidates")
                .actual(format!("duplicate id `{}` at paths `{}` and `{}`", c1.id, c1.path, c2.path))
                .fix(FixAction::safe(
                    "skill.add.specify_path",
                    "Use --path to specify the exact skill to install.",
                )));
            }
        }
    }

    // 6. Select skill(s) to install
    let chosen: Vec<&CandidateSkill> = if filtered.len() == 1 {
        filtered.first().into_iter().collect()
    } else {
        let select_options: Vec<SelectOption> = filtered
            .iter()
            .map(|c| SelectOption {
                id: c.id.clone(),
                description: c.description.clone(),
                path_if_any: c.path.clone(),
            })
            .collect();

        let prompt = format!("Multiple skills found in {}:", ext.repo);
        let selected_indices = ctx.confirm.select_many(&prompt, &select_options)?;
        if selected_indices.is_empty() {
            return Err(Failure::blocked(
                "skill.add.none_selected",
                "no skills were selected for installation",
            ));
        }

        let mut chosen = Vec::new();
        for &idx in &selected_indices {
            if let Some(c) = filtered.get(idx) {
                chosen.push(c);
            }
        }
        chosen
    };

    // 7. Refuse if any chosen skill already exists in either root
    for c in &chosen {
        if let Some((dir, _root)) = super::enumerate::resolve(&layout, &c.id)? {
            return Err(Failure::blocked(
                "skill.add.already_exists",
                format!("skill `{}` already exists at `{}`", c.id, dir),
            )
            .expected(format!(
                "skill `{}` to not already exist in either skills root",
                c.id
            ))
            .actual(format!("skill directory already present at `{dir}`"))
            .fix(FixAction::safe(
                "skill.remove",
                format!(
                    "Run `ivar skill remove {}` first if you want to replace it.",
                    c.id
                ),
            )));
        }
    }

    // 8. Install each chosen skill
    let mut installed = Vec::new();
    let target_root = if input.hall {
        layout.hall_skills()
    } else {
        layout.hall_skills_local()
    };

    for c in chosen {
        let target_dir = target_root.join(&c.id);
        fs::copy_dir(&c.dir, &target_dir).map_err(|e| {
            Failure::failed(
                "skill.add.copy_failed",
                format!("could not copy skill `{}`: {e}", c.id),
            )
        })?;

        let skill_file = target_dir.join("SKILL.md");
        if fs::exists(&skill_file)? {
            let raw = fs::read_text(&skill_file)?.unwrap_or_default();
            let mut fm = crate::store::skill::parse_frontmatter(&raw)
                .ok()
                .flatten()
                .unwrap_or_else(|| crate::domain::skill::SkillFrontmatter {
                    name: c.id.clone(),
                    description: c.description.clone(),
                    source: None,
                });

            fm.source = Some(ExternalRef {
                repo: ext.repo.clone(),
                path: c.path.clone(),
                git_ref: ext.git_ref.clone(),
            });

            let new_raw = frontmatter::replace(&raw, &fm).map_err(|e| {
                Failure::failed(
                    "skill.add.frontmatter_error",
                    format!("could not serialize frontmatter: {e}"),
                )
            })?;

            fs::write_text(&skill_file, &new_raw).map_err(|e| {
                Failure::failed(
                    "skill.add.write_error",
                    format!("could not update SKILL.md: {e}"),
                )
            })?;
        }

        let repo_id = RepoName::new(&c.id).map_err(|e| {
            Failure::failed(
                "skill.add.invalid_id",
                format!("invalid skill id `{}`: {e}", c.id),
            )
        })?;

        installed.push(InstalledSkill {
            id: repo_id,
            skill_file,
        });
    }

    Ok(Report::new(AddOutcome {
        root: layout.root().to_path_buf(),
        skills: installed,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/skill/add.rs"]
mod tests;
