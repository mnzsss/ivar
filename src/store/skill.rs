//! Skill installation state — the lockfile that records what is installed.
//!
//! This module owns the on-disk persistence for the sync planner's recorded
//! state. It is a thin wrapper around [`infra::json`], providing typed read /
//! write paths under `<hall>/.ivar/skills/state.json`.
//!
//! # Contract
//!
//! - `read(layout)` — absent is `Ok(None)`; present-but-unparseable is an error.
//! - `write(layout, &state)` — canonical JSON, atomic, idempotent.
//! - The state file lives alongside skills so it is versioned with them when
//!   the hall's `.gitignore` re-includes `.ivar/skills/`.
//!
//! SKILL.md parsing also lives here rather than in `domain::skill` — it needs
//! `infra::frontmatter` and `infra::fs`, and `domain` may not import `infra`.
//! The pure construction stays in the domain; the I/O wrapper is this module's.

use camino::Utf8PathBuf;

use crate::domain::name::RepoName;
use crate::domain::skill::{Skill, SkillFrontmatter};
use crate::domain::skill_sync::{State, TargetId};
use crate::error::{Failure, FixAction};
use crate::infra::json;

/// Everything that can go wrong reading or writing the skill lockfile.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The underlying JSON operation failed (serialisation, parse, or I/O).
    #[error(transparent)]
    Json(#[from] json::Error),

    /// The filesystem would not answer a question this module had to ask it.
    #[error(transparent)]
    Fs(#[from] crate::infra::fs::Error),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        match error {
            Error::Json(source) => source.into(),
            Error::Fs(source) => source.into(),
        }
    }
}

/// Parse frontmatter from a SKILL.md file.
///
/// Extracts `name`, `description`, and optional `source` fields. Returns
/// `Ok(None)` when there is no frontmatter or the frontmatter has no `name`.
/// Returns `Err` only on malformed YAML or structural issues.
pub fn parse_frontmatter(content: &str) -> Result<Option<SkillFrontmatter>, Failure> {
    let split = crate::infra::frontmatter::split(content)?;
    let block = split.frontmatter.unwrap_or_default();

    if block.is_empty() {
        return Ok(None);
    }

    let fm: SkillFrontmatter = serde_saphyr::from_str(block)
        .map_err(|e| Failure::failed("skill.parse_error", format!("malformed frontmatter: {e}")))?;

    // Require `name` — a skill without a name is not usable.
    if fm.name.trim().is_empty() {
        return Err(Failure::blocked(
            "skill.missing_name",
            "SKILL.md frontmatter must include a non-empty `name` field",
        )
        .expected("a `name` field in frontmatter")
        .actual("no `name` field found"));
    }

    Ok(Some(fm))
}

/// Parse a single SKILL.md and return a [`Skill`] if it is well-formed.
///
/// This is the full pipeline: read frontmatter → validate → construct.
/// Returns `Ok(None)` when the file has no frontmatter or no `name`,
/// and `Err` on hard failures (unparseable YAML, bad directory name).
pub fn parse_skill(dir: camino::Utf8PathBuf) -> Result<Option<Skill>, Failure> {
    let id = dir.file_name().ok_or_else(|| {
        Failure::blocked("skill.bad_dir", format!("directory has no name: {dir}"))
    })?;

    let id: RepoName = RepoName::new(id).map_err(|_e| {
        Failure::blocked(
            "skill.invalid_id",
            format!("invalid skill directory name: {id}"),
        )
        .fix(FixAction::safe(
            "skill.rename_dir",
            "Rename the directory to a valid single path segment.",
        ))
    })?;

    let Some(raw) = crate::infra::fs::read_text(&dir.join("SKILL.md")).map_err(|e| {
        Failure::failed(
            "skill.read_error",
            format!("could not read SKILL.md in {dir}: {e}"),
        )
    })?
    else {
        return Err(Failure::failed(
            "skill.missing_file",
            format!("SKILL.md not found in {dir}"),
        ));
    };

    let Some(fm) = parse_frontmatter(&raw)? else {
        return Ok(None);
    };

    Ok(Some(Skill::from_frontmatter(id, dir, fm)))
}

/// Read the skill installation state from the hall's skills directory.
///
/// Returns `Ok(None)` when the state file does not exist yet — the normal case
/// for a fresh hall. A present-but-unparseable file is a hard error.
pub fn read(hall_root: &camino::Utf8Path) -> Result<Option<State>, Error> {
    let path = hall_root.join(".ivar").join("skills").join("state.json");
    Ok(json::read(&path)?)
}

/// Write `state` to the hall's skills lockfile atomically.
///
/// Uses canonical JSON (sorted keys, two-space indent, trailing newline).
/// Creates the parent directory if it does not exist.
pub fn write(hall_root: &camino::Utf8Path, state: &State) -> Result<(), Error> {
    let dir = hall_root.join(".ivar").join("skills");
    crate::infra::fs::ensure_dir(&dir)?;
    json::write_canonical(&dir.join("state.json"), state)?;
    Ok(())
}

/// Build a target path for a given skill and provider.
///
/// Authored skills → `<provider>/skills/<id>` (e.g. `.claude/skills/my-skill`).
/// External skills use the same pattern — the renderer decides whether to
/// symlink or copy.
pub fn target_path(provider: TargetId, skill_id: &str) -> Utf8PathBuf {
    let config_dir = provider_config_dir(provider);
    Utf8PathBuf::from(config_dir).join("skills").join(skill_id)
}

/// The harness-specific config directory name.
fn provider_config_dir(provider: TargetId) -> &'static str {
    match provider {
        TargetId::Claude => ".claude",
        TargetId::OpenCode => ".opencode",
    }
}

#[cfg(test)]
#[path = "../../tests/unit/store/skill.rs"]
mod tests;
