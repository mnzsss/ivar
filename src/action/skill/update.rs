//! `ivar skill update [skills...]` — update external skills to their tracked ref.
//!
//! For each requested skill:
//! - **Authored** (local): no-op with a warning explaining why.
//! - **External**: re-download from the upstream repo and replace contents.
//!
//! One failing skill does not abort the batch — failures are collected as
//! warnings inside the `Ok` channel.

use std::io;
use std::io::Write;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::error::{Failure, Outcome, Report, Warning, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;
use crate::action::Ctx;

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
    let skills_dir = layout.hall_skills();

    // If there is no skills directory at all, nothing to do — clean result.
    if !fs::exists(&skills_dir)? {
        return Ok(Report::new(UpdateOutcome {
            root: layout.root().to_path_buf(),
            processed: 0,
        }));
    }

    let mut warnings = Vec::new();
    let mut processed = 0usize;

    for skill_name in &input.skills {
        let skill_dir = skills_dir.join(skill_name);
        let skill_file = skill_dir.join("SKILL.md");

        // If the skill directory doesn't exist, skip it silently.
        if !fs::exists(&skill_file)? {
            continue;
        }

        let raw = fs::read_text(&skill_file)?.ok_or_else(|| {
            Failure::failed(
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
                match try_download_and_extract(&layout, &skill_dir, &fm) {
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
    _layout: &crate::store::layout::Layout,
    skill_dir: &camino::Utf8Path,
    fm: &crate::domain::skill::SkillFrontmatter,
) -> Result<(), String> {
    let ext = fm.source.as_ref().unwrap();

    // Fetch tarball from GitHub and extract it into the skill directory.
    // Uses the infra::github helpers for auth + network.
    match crate::infra::github::fetch_tarball(&ext.repo, &ext.git_ref) {
        Ok(tarball_bytes) => {
            extract_tarball_into(&tarball_bytes, skill_dir).map_err(|e| e.to_string())
        }
        Err(e) => Err(format!("{e}")),
    }
}

/// Extract a gzipped tarball bytes into the target directory using system `tar`.
fn extract_tarball_into(data: &[u8], target_dir: &camino::Utf8Path) -> std::io::Result<()> {
    let mut child = std::process::Command::new("tar")
        .args(["xzf", "-"])
        .current_dir(target_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(data)?;
    }

    let status = child.wait()?;

    if !status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "tar extraction failed",
        ));
    }

    Ok(())
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

    fn write_skill(root: &camino::Utf8Path, id: &str, source: Option<&str>) {
        let dir = Layout::at(root.to_path_buf()).hall_skills().join(id);
        fs::ensure_dir(&dir).unwrap();
        let source_block = if let Some(repo) = source {
            format!("\nsource:\n  repo: \"{repo}\"\n  path: \"skills/{id}\"\n  ref: \"main\"")
        } else {
            String::new()
        };
        fs::write_text(
            &dir.join("SKILL.md"),
            &format!(
                "---\nname: {id}\ndescription: test skill{src}\n---\n\nbody\n",
                src = source_block
            ),
        )
        .unwrap();
    }

    #[test]
    fn update_authored_skill_is_a_noop_with_warning() {
        let (_guard, root) = seeded_hall();
        write_skill(&root, "refactor", None);

        let ctx = Ctx::new(root);
        let report = update(
            &ctx,
            UpdateInput {
                skills: vec!["refactor".to_owned()],
            },
        )
        .unwrap();

        assert_eq!(report.value.processed, 1);
        assert!(!report.warnings.is_empty());
        assert_eq!(report.warnings[0].code, "skill.update.authored_noop");
        assert_eq!(report.warnings[0].subject, "refactor");
    }

    #[test]
    fn update_external_skill_attempts_download() {
        let (_guard, root) = seeded_hall();
        write_skill(&root, "external-skill", Some("owner/toolkit"));

        let ctx = Ctx::new(root);
        let report = update(
            &ctx,
            UpdateInput {
                skills: vec!["external-skill".to_owned()],
            },
        )
        .unwrap();

        // The download will fail (no real network), but it should be recorded
        // as a warning, not a hard error.
        assert_eq!(report.value.processed, 1);
        assert!(!report.warnings.is_empty());
        assert_eq!(report.warnings[0].code, "skill.update.download_failed");
    }

    #[test]
    fn one_failing_skill_does_not_abort_the_batch() {
        let (_guard, root) = seeded_hall();
        // Authored skill — no-op (always succeeds)
        write_skill(&root, "authored", None);
        // External skill — will fail to download
        write_skill(&root, "external", Some("owner/toolkit"));

        let ctx = Ctx::new(root);
        let report = update(
            &ctx,
            UpdateInput {
                skills: vec!["authored".to_owned(), "external".to_owned()],
            },
        )
        .unwrap();

        // Both skills were processed.
        assert_eq!(report.value.processed, 2);
        // Two warnings: one authored_noop, one download_failed.
        assert_eq!(report.warnings.len(), 2);
        assert_eq!(report.warnings[0].code, "skill.update.authored_noop");
        assert_eq!(report.warnings[1].code, "skill.update.download_failed");
    }

    #[test]
    fn update_of_nonexistent_skill_is_clean() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root);

        let report = update(
            &ctx,
            UpdateInput {
                skills: vec!["nonexistent".to_owned()],
            },
        )
        .unwrap();

        assert!(report.is_clean());
        assert_eq!(report.value.processed, 0);
    }
}
