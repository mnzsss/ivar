//! Pure sync-plan reducer for skills.
//!
//! This module contains no I/O. Callers feed it the declared skills, the
//! materialised target state, and the recorded installation state; it returns
//! the steps required to bring the target in line with the declaration.
//!
//! Ported from `valhalla/packages/ragnar/src/sync-plan.ts`.

use std::collections::HashMap;

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use crate::domain::name::RepoName;
use crate::domain::provider::Provider;
use crate::domain::skill::{RenderMode, Skill};

/// The target harness a skill can be materialised for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetId {
    Claude,
    OpenCode,
    Omp,
}

impl TargetId {
    /// The canonical string identifier used in state files.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::OpenCode => "opencode",
            Self::Omp => "omp",
        }
    }
}

impl From<Provider> for TargetId {
    fn from(provider: Provider) -> Self {
        match provider {
            Provider::ClaudeCode => Self::Claude,
            Provider::OpenCode => Self::OpenCode,
            Provider::Omp => Self::Omp,
        }
    }
}

/// What the planner decided to do about one skill/target pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Materialise the skill at the target path.
    Create,
    /// Replace an existing materialisation with the correct one.
    Update,
    /// Remove a materialisation that is no longer declared.
    Remove,
    /// The target already matches the declaration.
    Unchanged,
}

/// One row of the sync plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// The skill being planned.
    pub skill: RepoName,
    /// Which provider target this step affects.
    pub target_id: TargetId,
    /// The absolute path this step affects.
    pub target: Utf8PathBuf,
    /// The absolute source path the target should point at or duplicate.
    pub source: Utf8PathBuf,
    /// What to do.
    pub action: Action,
    /// How the target should be materialised.
    pub mode: RenderMode,
    /// Why this action was chosen, when it is not obvious.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The materialised state of one skill for one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Which target this state describes.
    pub id: TargetId,
    /// The skill this target belongs to.
    pub skill: RepoName,
    /// The target path on disk.
    pub path: Utf8PathBuf,
    /// The source directory the target should match.
    pub source_path: Utf8PathBuf,
    /// The current tree hash of [`Self::source_path`].
    pub source_hash: String,
    /// What is actually at [`Self::path`] right now.
    pub status: MaterialStatus,
}

/// What the renderer found at a target path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterialStatus {
    /// Nothing exists at the path.
    Missing,
    /// The path matches the expected source.
    Ok,
    /// The path is a symlink but points somewhere else.
    WrongLink,
    /// The path exists but is not a symlink (for symlink mode) or is a symlink
    /// (for copy mode).
    NotLink,
    /// The path is a symlink whose target no longer exists.
    BrokenSymlink,
}

/// One recorded provider entry in the installation state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEntry {
    pub target_path: Utf8PathBuf,
    pub rendered_hash: String,
    pub linked_at: String,
    pub mode: Option<RenderMode>,
}

/// One recorded installation entry in the installation state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallationEntry {
    pub source_path: Utf8PathBuf,
    pub source_hash: String,
    pub installed_at: String,
    pub commit_sha: Option<String>,
    pub providers: HashMap<TargetId, ProviderEntry>,
}

/// The recorded installation state for the hall.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct State {
    pub installations: HashMap<String, InstallationEntry>,
}

/// Optional optimisations for the planner.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanOptions {
    /// The current HEAD commit SHA of the hall's repository, if known.
    pub repo_head: Option<String>,
    /// Whether the hall's working tree is clean.
    pub tree_clean: bool,
}

/// Plan the steps needed to reconcile declared skills with recorded state and
/// materialised targets.
///
/// `targets` holds one entry per (skill, target id); every one is planned.
/// Orphans — recorded targets of undeclared skills, and recorded targets of
/// providers not in `available` — are planned exactly once.
pub fn plan(
    skills: &[Skill],
    targets: &[Target],
    state: &State,
    available: &[TargetId],
) -> Vec<Step> {
    plan_with_options(skills, targets, state, available, &PlanOptions::default())
}

/// Plan with optional commit-sha short-circuit.
pub fn plan_with_options(
    skills: &[Skill],
    targets: &[Target],
    state: &State,
    available: &[TargetId],
    opts: &PlanOptions,
) -> Vec<Step> {
    let skill_by_id: HashMap<&str, &Skill> = skills
        .iter()
        .map(|skill| (skill.id.as_str(), skill))
        .collect();

    let mut steps: Vec<Step> = targets
        .iter()
        .filter_map(|target| {
            let skill = skill_by_id.get(target.skill.as_str())?;
            Some(plan_for_skill(skill, target, state, opts))
        })
        .collect();

    steps.extend(plan_orphans(skills, state, available));

    steps.sort_by(|a, b| {
        (a.skill.as_str(), a.target.as_str()).cmp(&(b.skill.as_str(), b.target.as_str()))
    });
    steps
}

pub fn plan_orphans(skills: &[Skill], state: &State, available: &[TargetId]) -> Vec<Step> {
    let declared: HashMap<&str, &Skill> = skills
        .iter()
        .map(|skill| (skill.id.as_str(), skill))
        .collect();
    let mut steps = Vec::new();

    for (id, entry) in &state.installations {
        let Ok(skill) = RepoName::new(id) else {
            continue;
        };
        let is_declared = declared.contains_key(id.as_str());
        for (target_id, provider) in &entry.providers {
            if is_declared && available.contains(target_id) {
                continue;
            }
            steps.push(Step {
                skill: skill.clone(),
                target_id: *target_id,
                target: provider.target_path.clone(),
                source: declared
                    .get(id.as_str())
                    .map_or_else(|| entry.source_path.clone(), |skill| skill.dir.clone()),
                action: Action::Remove,
                mode: provider.mode.unwrap_or_else(RenderMode::default_mode),
                reason: None,
            });
        }
    }
    steps
}

fn plan_for_skill(skill: &Skill, target: &Target, state: &State, opts: &PlanOptions) -> Step {
    let mode = skill.render_mode();
    let entry = state.installations.get(skill.id.as_str());
    let provider = entry
        .as_ref()
        .and_then(|entry| entry.providers.get(&target.id));
    let base = Step {
        skill: skill.id.clone(),
        target_id: target.id,
        target: target.path.clone(),
        source: target.source_path.clone(),
        action: Action::Unchanged,
        mode,
        reason: None,
    };

    // Commit-sha short-circuit: if the hall is at a clean commit we have
    // already recorded, nothing has changed.
    if let Some(repo_head) = &opts.repo_head
        && opts.tree_clean
        && entry.as_ref().and_then(|entry| entry.commit_sha.as_deref()) == Some(repo_head)
    {
        return Step {
            reason: Some("commit-sha unchanged".to_owned()),
            ..base
        };
    }

    if let Some(provider) = provider {
        let prev_mode = provider.mode.unwrap_or_else(RenderMode::default_mode);
        if prev_mode != mode && target.status != MaterialStatus::Missing {
            return Step {
                action: Action::Update,
                reason: Some("render mode changed".to_owned()),
                ..base
            };
        }

        match target.status {
            MaterialStatus::Missing => Step {
                action: Action::Create,
                ..base
            },
            MaterialStatus::WrongLink | MaterialStatus::BrokenSymlink => Step {
                action: Action::Update,
                ..base
            },
            MaterialStatus::NotLink => Step {
                action: Action::Update,
                reason: Some("target is not a symlink".to_owned()),
                ..base
            },
            MaterialStatus::Ok => match entry {
                Some(entry) if entry.source_hash != target.source_hash => Step {
                    action: Action::Unchanged,
                    reason: Some("source changed (hash refreshed)".to_owned()),
                    ..base
                },
                _ => base,
            },
        }
    } else {
        match target.status {
            MaterialStatus::Missing => Step {
                action: Action::Create,
                ..base
            },
            MaterialStatus::Ok => Step {
                action: Action::Create,
                reason: Some("adopt existing target".to_owned()),
                ..base
            },
            MaterialStatus::WrongLink | MaterialStatus::BrokenSymlink | MaterialStatus::NotLink => {
                Step {
                    action: Action::Update,
                    reason: Some("untracked target entry exists".to_owned()),
                    ..base
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/domain/skill_sync.rs"]
mod tests;
