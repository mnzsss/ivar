use clap::{Args, Subcommand};

use crate::action::skill::{
    add as skill_add, create as skill_create, detach as skill_detach, remove as skill_remove,
    toggle as skill_toggle, update as skill_update,
};

/// The `ivar skill` surface: the hall's shared skills directory.
#[derive(Debug, Subcommand)]
pub enum SkillCommand {
    /// List the skills in the hall's shared skills directory.
    List,
    /// Scaffold a new skill: a folder with a SKILL.md.
    Create(SkillCreateArgs),
    /// Install an external skill from a git repo.
    Add(SkillAddArgs),
    /// Update external skills to their tracked ref.
    Update(SkillUpdateArgs),
    /// Remove a skill from the hall's shared skills directory.
    Remove(SkillRemoveArgs),
    /// Convert an external skill into an authored (local) skill.
    Detach(SkillDetachArgs),
    /// Turn a skill back on and materialise it for the hall's providers.
    Enable(SkillToggleArgs),
    /// Turn a skill off without deleting it: its provider targets are removed
    /// and `ivar skill sync` leaves it out until it is enabled again. The
    /// choice is personal (`.ivar/skills-local/disabled.json`) and applies
    /// from the next session.
    Disable(SkillToggleArgs),
    /// Materialise hall skills for the hall's providers and remove the ones
    /// left behind by providers it no longer lists.
    Sync,
    /// Show skill installation state — which are external, authored, or stale.
    Status,
    /// Health diagnostics for skills: find broken links, missing refs, and
    /// suggest fix_actions.
    Doctor,
}

/// Arguments for `ivar skill create`.
#[derive(Debug, Args)]
pub struct SkillCreateArgs {
    /// The skill's id — one path segment, unique across both skills roots.
    pub id: String,
    /// The skill's description, for the SKILL.md frontmatter.
    #[arg(long)]
    pub description: String,
    /// Create it in the hall's committed skills directory, shared with
    /// everyone who clones the hall. Without this it stays personal to you.
    #[arg(long)]
    pub hall: bool,
}

/// Arguments for `ivar skill add`.
#[derive(Debug, Args)]
pub struct SkillAddArgs {
    /// The skill source: owner/repo, https://github.com/owner/repo, or
    /// https://github.com/owner/repo/tree/<ref>/<path>.
    pub repo: String,
    /// A sub-path inside the repo that holds the skill folder.
    #[arg(long)]
    pub path: Option<String>,
    /// A git ref (branch, tag, or sha) to pin the skill to.
    #[arg(long)]
    pub r#ref: Option<String>,
    /// Install into the hall's committed skills directory, shared with
    /// everyone who clones the hall. Without this it stays personal to you.
    #[arg(long)]
    pub hall: bool,
}

/// Arguments for `ivar skill update`.
#[derive(Debug, Args)]
pub struct SkillUpdateArgs {
    /// Which external skills to update; updates all when omitted.
    pub skills: Vec<String>,
}

/// Arguments for `ivar skill remove`.
#[derive(Debug, Args)]
pub struct SkillRemoveArgs {
    /// The skill's id to remove.
    pub skill: String,
}

/// Arguments for `ivar skill detach`.
#[derive(Debug, Args)]
pub struct SkillDetachArgs {
    /// The external skill's id to convert into an authored skill.
    pub skill: String,
}

impl From<SkillCreateArgs> for skill_create::CreateInput {
    fn from(args: SkillCreateArgs) -> Self {
        let SkillCreateArgs {
            id,
            description,
            hall,
        } = args;
        Self {
            id,
            description,
            hall,
        }
    }
}

impl From<SkillAddArgs> for skill_add::AddInput {
    fn from(args: SkillAddArgs) -> Self {
        let SkillAddArgs {
            repo,
            path,
            r#ref,
            hall,
        } = args;
        Self {
            repo,
            path,
            ref_: r#ref,
            hall,
        }
    }
}

impl From<SkillUpdateArgs> for skill_update::UpdateInput {
    fn from(args: SkillUpdateArgs) -> Self {
        let SkillUpdateArgs { skills } = args;
        Self { skills }
    }
}

impl From<SkillRemoveArgs> for skill_remove::RemoveInput {
    fn from(args: SkillRemoveArgs) -> Self {
        let SkillRemoveArgs { skill } = args;
        Self { skill }
    }
}

impl From<SkillDetachArgs> for skill_detach::DetachInput {
    fn from(args: SkillDetachArgs) -> Self {
        let SkillDetachArgs { skill } = args;
        Self { skill }
    }
}

/// Arguments for `ivar skill enable` and `ivar skill disable`.
#[derive(Debug, Args)]
pub struct SkillToggleArgs {
    /// The skill's id, from either skills root.
    pub skill: String,
}

impl From<SkillToggleArgs> for skill_toggle::ToggleInput {
    fn from(args: SkillToggleArgs) -> Self {
        let SkillToggleArgs { skill } = args;
        Self { skill }
    }
}
