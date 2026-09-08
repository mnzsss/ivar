//! The shipped skill catalog: every official workflow skill, embedded at
//! compile time, paired with the legacy fingerprint of the skill it supersedes.
//!
//! The catalog is the *source*; reconciliation lives in
//! [`super`]'s `materialise` / `remove` / `inspect`. This file owns only the
//! declarative data and its accessors.

/// One shipped workflow skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShippedSkill {
    /// The skill's id — the `<id>` in `ivar-<id>/SKILL.md` and in `/ivar-<id>`.
    pub id: &'static str,
    /// The provider-neutral Markdown source, embedded at compile time.
    pub content: &'static str,
    /// SHA-256 of the legacy skill file this id supersedes (if any).
    pub legacy_sha256: Option<&'static str>,
}

impl ShippedSkill {
    /// The directory name this skill materialises into: `ivar-<id>`.
    #[must_use]
    pub fn skill_dir_name(self) -> String {
        format!("ivar-{}", self.id)
    }

    /// The relative path to the skill file: `ivar-<id>/SKILL.md`.
    #[must_use]
    pub fn skill_file_rel_path(self) -> String {
        format!("ivar-{}/SKILL.md", self.id)
    }

    /// The legacy directory name this skill supersedes: `<id>`.
    #[must_use]
    pub fn legacy_dir_name(self) -> String {
        self.id.to_owned()
    }
}

/// Every shipped workflow skill, in a stable order.
pub const fn catalog() -> &'static [ShippedSkill] {
    SKILLS
}

const SKILLS: &[ShippedSkill] = &[ShippedSkill {
    id: "execute",
    content: include_str!("ivar-execute/SKILL.md"),
    legacy_sha256: None,
}];
