//! `ivar skill update [skills...]` — update external skills to their tracked ref.
//!
//! For each requested skill:
//! - **Authored** (local): no-op with a warning explaining why.
//! - **External**: re-download from the upstream repo and replace contents.
//!
//! One failing skill does not abort the batch — failures are collected as
//! warnings inside the `Ok` channel.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::error::{Outcome, Report, Warning, WriteHuman};
use crate::infra::frontmatter;
use crate::infra::fs;

use super::super::discover_hall;

/// What `ivar skill update` needs.
#[derive(Debug, Clone)]
pub struct UpdateInput {
    pub skills: Vec<String>,
}

/// What `ivar skill update` did.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// Number of skills processed.
    pub processed: usize,
}

impl WriteHuman for UpdateOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Updated {} skill(s) in {}", self.processed, self.root)
    }
}

/// Update external skills to their tracked ref.
///
/// Authored skills are left alone with a warning. External skills are
/// re-downloaded from their upstream repo. One failure does not abort the batch.
pub fn update(ctx: &Ctx, input: UpdateInput) -> Outcome<UpdateOutcome> {
    let layout = discover_hall(ctx)?;

    let mut warnings = Vec::new();
    let mut processed = 0usize;

    for skill_name in &input.skills {
        // Look in both roots — a personal skill can be external too, and
        // updating it must work without naming its root.
        let Some((skill_dir, _root)) = super::enumerate::resolve(&layout, skill_name)? else {
            continue;
        };
        let skill_file = skill_dir.join("SKILL.md");

        // A directory without a SKILL.md is not a skill; skip it silently.
        if !fs::exists(&skill_file)? {
            continue;
        }

        let raw = fs::read_text(&skill_file)?.ok_or_else(|| {
            crate::error::Failure::failed(
                "skill.update.read",
                format!("could not read SKILL.md for skill `{skill_name}`"),
            )
        })?;

        let fm = match crate::store::skill::parse_frontmatter(&raw) {
            Ok(Some(fm)) => fm,
            Ok(None) => {
                warnings.push(Warning::new(
                    "skill.update.no_source",
                    skill_name,
                    "no frontmatter found — skipping",
                ));
                processed += 1;
                continue;
            }
            Err(e) => {
                warnings.push(Warning::new(
                    "skill.update.parse_error",
                    skill_name,
                    format!("could not parse frontmatter: {e}"),
                ));
                processed += 1;
                continue;
            }
        };

        match fm.source {
            Some(_) => {
                // Attempt to re-download and extract from upstream.
                // This is best-effort — a failed download produces a warning, not a hard error.
                match try_download_and_extract(&skill_dir, &fm) {
                    Ok(()) => {}
                    Err(e) => {
                        warnings.push(Warning::new(
                            "skill.update.download_failed",
                            skill_name,
                            format!("could not update: {e}"),
                        ));
                    }
                }
                processed += 1;
            }
            None => {
                // Authored skill — no-op.
                warnings.push(Warning::new(
                    "skill.update.authored_noop",
                    skill_name,
                    "authored skill — update is a no-op",
                ));
                processed += 1;
            }
        }
    }

    let report = Report::new(UpdateOutcome {
        root: layout.root().to_path_buf(),
        processed,
    });

    if warnings.is_empty() {
        Ok(report)
    } else {
        Ok(Report::with_warnings(report.value, warnings))
    }
}

/// Best-effort: fetch the latest from the upstream repo and extract into `skill_dir`.
///
/// Returns `Err` on network or extraction failure — the caller converts this to
/// a warning so one bad download never aborts the batch.
fn try_download_and_extract(
    skill_dir: &camino::Utf8Path,
    fm: &crate::domain::skill::SkillFrontmatter,
) -> Result<(), String> {
    let Some(ext) = fm.source.as_ref() else {
        return Err("skill has no external source".to_owned());
    };

    // Fetch tarball from GitHub and extract it into a temp directory.
    let tarball_bytes =
        crate::infra::github::fetch_tarball(&ext.repo, &ext.git_ref).map_err(|e| e.to_string())?;

    let temp_dir = fs::TempDir::new().map_err(|e| e.to_string())?;
    super::extract_tarball_into(&tarball_bytes, temp_dir.path()).map_err(|e| e.to_string())?;

    let repo_root = find_repo_root(temp_dir.path());
    let source_dir = if ext.path.is_empty() {
        repo_root
    } else {
        repo_root.join(&ext.path)
    };

    if !fs::exists(&source_dir).unwrap_or(false) {
        return Err(format!(
            "recorded path `{}` not found in repository `{}`",
            ext.path, ext.repo
        ));
    }

    // Replace skill_dir contents with source_dir contents.
    fs::remove_path(skill_dir).map_err(|e| e.to_string())?;
    fs::copy_dir(&source_dir, skill_dir).map_err(|e| e.to_string())?;

    // Re-inject the frontmatter source field into SKILL.md.
    let skill_file = skill_dir.join("SKILL.md");
    if let Ok(Some(raw)) = fs::read_text(&skill_file) {
        let mut updated_fm = crate::store::skill::parse_frontmatter(&raw)
            .ok()
            .flatten()
            .unwrap_or_else(|| fm.clone());
        updated_fm.source = Some(ext.clone());
        if let Ok(new_raw) = frontmatter::replace(&raw, &updated_fm) {
            let _ = fs::write_text(&skill_file, &new_raw);
        }
    }

    Ok(())
}

fn find_repo_root(temp_dir: &camino::Utf8Path) -> camino::Utf8PathBuf {
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

#[cfg(test)]
#[path = "../../../tests/unit/action/skill/update.rs"]
mod tests;
