//! Domain types for skills — the hall's shared skill definitions.
//!
//! A **skill** is a folder under `<hall>/.ivar/skills/` whose `SKILL.md` carries
//! YAML frontmatter with at least a `name` field. The folder's basename is the
//! skill's id. Skills may be authored locally (no external source) or declared
//! as stubs pointing to an upstream repository.
//!
//! # Contract
//!
//! - [`Skill`] — the parsed skill, built from frontmatter. Authored skills carry
//!   no external source; external stubs carry one via [`ExternalRef`].
//! - [`Source`] — whether a skill is authored here or points to an upstream repo.
//! - [`ExternalRef`] — the repo, path, and git ref an external skill is fetched
//!   from.
//! - Frontmatter parsing lives in this module because it is pure data
//!   transformation — no I/O, no network, just text → types.
//!
//! # Scope decision (R-3)
//!
//! Two roots are supported: committed hall skills (`<hall>/.ivar/skills/`) and
//! local, personal skills (`<hall>/.ivar/skills-local/`). User-home
//! skills and cross-hall skills remain out of scope for now.
//!
/// Where a skill definition lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillRoot {
    /// The hall's committed skills directory.
    Hall,
    /// The hall's local, personal, gitignored skills directory.
    Local,
}

use serde::{Deserialize, Serialize};

use crate::domain::name::RepoName;

/// Whether a skill is materialised as a symlink (authored) or copied (external).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderMode {
    /// Symlink to the source directory. Used for authored skills.
    #[default]
    Symlink,
    /// Copy the source directory contents. Used for external skills.
    Copy,
}

impl RenderMode {
    /// The default mode for a skill without explicit source configuration.
    #[must_use]
    pub fn default_mode() -> Self {
        Self::Symlink
    }
}

/// An external skill source: where to fetch the skill from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalRef {
    /// The upstream repository, e.g. `"owner/repo"`.
    pub repo: String,
    /// The path inside the repository where the skill lives, e.g. `"skills/my-skill"`.
    pub path: String,
    /// The git reference (branch, tag, or commit SHA).
    #[serde(rename = "ref")]
    pub git_ref: String,
}

/// Where a skill comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    /// Authored locally in the hall's skills directory.
    Authored,
    /// Points to an upstream repository.
    External(ExternalRef),
}

/// Frontmatter parsed from a skill's `SKILL.md`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillFrontmatter {
    /// The human-readable name of the skill. Required (validated after parse).
    pub name: String,
    /// A short description. `None` when absent or empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// External source, if any. `None` means the skill is authored locally.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ExternalRef>,
}

impl SkillFrontmatter {
    /// Create frontmatter for an authored (local) skill.
    #[must_use]
    pub fn authored(name: &str, description: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            description: Some(description.into()),
            source: None,
        }
    }
}

/// Alias for the external skill source — matches the TypeScript naming.
pub type ExternalSkillSource = ExternalRef;

/// A parsed skill definition.
///
/// Built from a directory containing a `SKILL.md` with valid frontmatter.
/// The `id` is derived from the directory basename; the `dir` is the full
/// path to that directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    /// The skill's unique identifier — the directory basename.
    pub id: RepoName,
    /// A short description of what the skill does.
    pub description: String,
    /// Where this skill comes from.
    pub source: Source,
    /// Which skills root defines this skill.
    pub root: SkillRoot,
    /// The absolute path to the directory containing this skill's files.
    pub dir: camino::Utf8PathBuf,
}

impl Skill {
    /// Build a skill from frontmatter and its source directory.
    ///
    /// The `id` is taken from the directory basename (validated as a
    /// [`RepoName`]). This is the internal constructor used by tests and by
    /// the full pipeline (`parse_skill`). For public-facing parsing from raw
    /// text, use [`parse_skill`] which returns `Result<Option<Self>, Failure>`.
    ///
    /// Panics if the id is invalid — callers that already have a `RepoName`
    /// guarantee are expected to use this path.
    pub fn from_frontmatter(
        id: RepoName,
        dir: camino::Utf8PathBuf,
        frontmatter: SkillFrontmatter,
        root: SkillRoot,
    ) -> Self {
        let description = frontmatter
            .description
            .unwrap_or_else(|| format!("The {} skill", id.as_str()));

        let source = match frontmatter.source {
            Some(ext) => Source::External(ext),
            None => Source::Authored,
        };

        Self {
            id,
            description,
            source,
            root,
            dir,
        }
    }

    /// How this skill should be materialised at targets.
    #[must_use]
    pub fn render_mode(&self) -> RenderMode {
        match &self.source {
            Source::Authored => RenderMode::Symlink,
            Source::External(_) => RenderMode::Copy,
        }
    }

    /// Whether this skill is authored locally.
    #[must_use]
    pub fn is_authored(&self) -> bool {
        matches!(&self.source, Source::Authored)
    }

    /// Whether this skill points to an external repository.
    #[must_use]
    pub fn is_external(&self) -> bool {
        matches!(&self.source, Source::External(_))
    }
}

/// Parse frontmatter from a SKILL.md file.
///
/// Extracts `name`, `description`, and optional `source` fields. Returns
/// `Ok(None)` when there is no frontmatter or the frontmatter has no `name`.
/// Returns `Err` only on malformed YAML or structural issues.
///
/// Lives in `store::skill` — not here — because splitting the frontmatter
/// block touches `infra::frontmatter`, and `domain` may not import `infra`.
/// This module keeps the pure [`Skill::from_frontmatter`] construction.
#[cfg(test)]
#[path = "../../tests/unit/domain/skill.rs"]
mod tests;
