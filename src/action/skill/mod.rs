//! `ivar skill` — the hall's shared skills, under `.ivar/skills/`.
//!
//! A skill is a folder whose `SKILL.md` carries `name` and `description`
//! frontmatter; the folder's basename is the skill's id. The skills dir is
//! one of the two committed children of `.ivar/` (see
//! [`crate::store::layout::gitignore_lines`]), so a team's skills survive
//! clones.
//!
//! This slice is local-only: list and create. No valhalla/ecbert sync — that
//! is a later integration, and the `.gitignore` already treats `.ivar/skills/`
//! as committed so a future `push`/`pull` has somewhere to put things.

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
pub mod update;

use std::io::Write;

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
