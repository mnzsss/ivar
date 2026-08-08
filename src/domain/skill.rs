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
//! Only hall-scoped skills (`<hall>/.ivar/skills/`) are supported. User-home
//! skills are out of scope for now.

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl Default for SkillFrontmatter {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: None,
            source: None,
        }
    }
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
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;

    // -- Skill construction ---------------------------------------------------

    #[test]
    fn authored_skill_has_no_source() {
        let skill = Skill::from_frontmatter(
            RepoName::new("alpha").unwrap(),
            camino::Utf8PathBuf::from("/skills/alpha"),
            SkillFrontmatter {
                name: "alpha".to_owned(),
                description: None,
                source: None,
            },
        );
        assert!(skill.is_authored());
        assert!(!skill.is_external());
        assert_eq!(skill.render_mode(), RenderMode::Symlink);
    }

    #[test]
    fn external_skill_has_an_external_source() {
        let skill = Skill::from_frontmatter(
            RepoName::new("beta").unwrap(),
            camino::Utf8PathBuf::from("/skills/beta"),
            SkillFrontmatter {
                name: "beta".to_owned(),
                description: Some("An external skill".to_owned()),
                source: Some(ExternalRef {
                    repo: "org/toolkit".to_owned(),
                    path: "skills/beta".to_owned(),
                    git_ref: "main".to_owned(),
                }),
            },
        );
        assert!(!skill.is_authored());
        assert!(skill.is_external());
        assert_eq!(skill.render_mode(), RenderMode::Copy);
    }

    #[test]
    fn default_description_is_generated_from_id() {
        let skill = Skill::from_frontmatter(
            RepoName::new("gamma").unwrap(),
            camino::Utf8PathBuf::from("/skills/gamma"),
            SkillFrontmatter {
                name: "gamma".to_owned(),
                description: None,
                source: None,
            },
        );
        assert_eq!(skill.description, "The gamma skill");
    }

    // -- Round-trip serialization ---------------------------------------------

    #[test]
    fn serialize_and_deserialize_authored_skill() {
        let original = Skill {
            id: RepoName::new("roundtrip").unwrap(),
            description: "A round-trip skill".to_owned(),
            source: Source::Authored,
            dir: camino::Utf8PathBuf::from("/skills/roundtrip"),
        };

        let json = serde_json::to_string(&original).unwrap();
        let restored: Skill = serde_json::from_str(&json).unwrap();

        assert_eq!(restored, original);
    }

    #[test]
    fn serialize_and_deserialize_external_skill() {
        let original = Skill {
            id: RepoName::new("ext-rt").unwrap(),
            description: "External skill".to_owned(),
            source: Source::External(ExternalRef {
                repo: "owner/repo".to_owned(),
                path: "skills/ext".to_owned(),
                git_ref: "abc123".to_owned(),
            }),
            dir: camino::Utf8PathBuf::from("/skills/ext-rt"),
        };

        let json = serde_json::to_string(&original).unwrap();
        let restored: Skill = serde_json::from_str(&json).unwrap();

        assert_eq!(restored, original);
    }

    #[test]
    fn render_mode_matches_source_type() {
        let authored = Skill {
            id: RepoName::new("a").unwrap(),
            description: "a".to_owned(),
            source: Source::Authored,
            dir: camino::Utf8PathBuf::from("/a"),
        };
        assert_eq!(authored.render_mode(), RenderMode::Symlink);

        let external = Skill {
            id: RepoName::new("b").unwrap(),
            description: "b".to_owned(),
            source: Source::External(ExternalRef {
                repo: "x/y".to_owned(),
                path: "p".to_owned(),
                git_ref: "z".to_owned(),
            }),
            dir: camino::Utf8PathBuf::from("/b"),
        };
        assert_eq!(external.render_mode(), RenderMode::Copy);
    }
}
