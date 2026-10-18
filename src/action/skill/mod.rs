//! `ivar skill` — the hall's shared skills, under `.ivar/skills/`.
//!
//! A skill is a folder whose `SKILL.md` carries `name` and `description`
//! frontmatter; the folder's basename is the skill's id. The skills dir is
//! one of the two committed children of `.ivar/` (see
//! [`crate::store::layout::gitignore_lines`]), so a team's skills survive
//! clones.
//!
//! Commands in this module manage hall and local skills: adding, creating,
//! detaching, listing, updating, synchronising to provider targets, checking
//! status, doctoring, and removing skills.

pub mod add;
pub mod create;
pub mod detach;
pub mod doctor;
pub mod enumerate;
pub mod list;
pub mod remove;
pub mod source;
pub mod status;
pub mod sync;
pub mod toggle;
pub mod update;

use std::io::Write;

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::{Failure, FixAction};
use crate::infra::fs;
use crate::store::layout::Layout;

use crate::domain::skill_sync::TargetId;
use crate::store::manifest::Manifest;

/// Native provider targets supported for skill materialisation.
pub(crate) const SKILL_TARGETS: [TargetId; 3] =
    [TargetId::Claude, TargetId::OpenCode, TargetId::Omp];

/// The skill targets of the hall's available providers, in `TargetId`
/// order (Claude, OpenCode, Omp). The manifest is the single source.
pub(crate) fn available_targets(manifest: &Manifest) -> Vec<TargetId> {
    SKILL_TARGETS
        .into_iter()
        .filter(|target| {
            manifest
                .providers()
                .available()
                .iter()
                .any(|provider| TargetId::from(*provider) == *target)
        })
        .collect()
}

/// Find the single top-level directory in `temp_dir` if present; otherwise `temp_dir`.
pub(super) fn find_repo_root(temp_dir: &Utf8Path) -> Utf8PathBuf {
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

/// The `skill.not_found` failure shared by every action that looks a skill
/// id up via [`enumerate::resolve`].
pub(super) fn skill_not_found(layout: &Layout, id: &str) -> Failure {
    Failure::blocked("skill.not_found", format!("skill `{id}` does not exist"))
        .expected("a skill directory in either skills root")
        .actual(format!(
            "no directory at `{}` or `{}`",
            layout.hall_skills_local().join(id),
            layout.hall_skills().join(id)
        ))
        .fix(FixAction::safe(
            "skill.list",
            "List available skills to find the correct id.",
        ))
}

/// Extract gzipped tarball bytes into `target_dir` using system `tar`.
pub(super) fn extract_tarball_into(
    data: &[u8],
    target_dir: &camino::Utf8Path,
) -> std::io::Result<()> {
    let mut child = std::process::Command::new("tar")
        .args(["xzf", "-"])
        .current_dir(target_dir.as_std_path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(data)?;
    }

    let status = child.wait()?;

    if !status.success() {
        return Err(std::io::Error::other("tar extraction failed"));
    }

    Ok(())
}
