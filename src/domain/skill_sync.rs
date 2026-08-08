//! Pure sync-plan reducer for skills.
//!
//! This module contains no I/O. Callers feed it the declared skills, the
//! materialised target state, and the recorded installation state; it returns
//! the steps required to bring the target in line with the declaration.
//!
//! Ported from `valhalla/packages/ragnar/src/sync-plan.ts`.

use std::collections::{HashMap, HashSet};

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use crate::domain::name::RepoName;
use crate::domain::skill::{RenderMode, Skill};

/// The target harness a skill can be materialised for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetId {
    Claude,
    OpenCode,
}

impl TargetId {
    /// The canonical string identifier used in state files.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::OpenCode => "opencode",
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
/// `targets` must contain exactly one entry per skill in `skills` for the
/// target being planned.
pub fn plan(skills: &[Skill], targets: &[Target], state: &State) -> Vec<Step> {
    plan_with_options(skills, targets, state, PlanOptions::default())
}

/// Plan with optional commit-sha short-circuit.
pub fn plan_with_options(
    skills: &[Skill],
    targets: &[Target],
    state: &State,
    opts: PlanOptions,
) -> Vec<Step> {
    let target_by_skill: HashMap<&str, &Target> = targets
        .iter()
        .map(|target| (target.skill.as_str(), target))
        .collect();

    let mut steps: Vec<Step> = skills
        .iter()
        .filter_map(|skill| {
            let target = target_by_skill.get(skill.id.as_str())?;
            Some(plan_for_skill(skill, target, state, opts.clone()))
        })
        .collect();

    steps.extend(plan_deletes(skills, state, targets));

    steps.sort_by(|a, b| a.skill.as_str().cmp(b.skill.as_str()));
    steps
}

fn plan_for_skill(skill: &Skill, target: &Target, state: &State, opts: PlanOptions) -> Step {
    let mode = skill.render_mode();
    let entry = state.installations.get(skill.id.as_str());
    let provider = entry
        .as_ref()
        .and_then(|entry| entry.providers.get(&target.id));
    let base = Step {
        skill: skill.id.clone(),
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

fn plan_deletes(skills: &[Skill], state: &State, targets: &[Target]) -> Vec<Step> {
    let declared: HashSet<&str> = skills.iter().map(|skill| skill.id.as_str()).collect();
    let target_by_skill: HashMap<&str, &Target> = targets
        .iter()
        .map(|target| (target.skill.as_str(), target))
        .collect();
    let mut steps = Vec::new();

    for (id, entry) in &state.installations {
        if declared.contains(id.as_str()) {
            continue;
        }
        let Ok(skill) = RepoName::new(id) else {
            continue;
        };
        for provider in entry.providers.values() {
            let target_path = provider.target_path.clone();
            let source_path = target_by_skill
                .get(id.as_str())
                .map(|target| target.source_path.clone())
                .unwrap_or_else(|| entry.source_path.clone());
            steps.push(Step {
                skill: skill.clone(),
                target: target_path,
                source: source_path,
                action: Action::Remove,
                mode: provider.mode.unwrap_or_else(RenderMode::default_mode),
                reason: None,
            });
        }
    }

    steps
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use std::collections::HashMap;

    use camino::Utf8PathBuf;

    use super::*;
    use crate::domain::name::RepoName;
    use crate::domain::skill::{ExternalSkillSource, Skill, SkillFrontmatter};

    fn authored_skill(id: &str, dir: &str) -> Skill {
        Skill::from_frontmatter(
            RepoName::new(id).unwrap(),
            Utf8PathBuf::from(dir),
            SkillFrontmatter::authored(id, format!("The {id} skill")),
        )
    }

    fn external_skill(id: &str, dir: &str) -> Skill {
        Skill::from_frontmatter(
            RepoName::new(id).unwrap(),
            Utf8PathBuf::from(dir),
            SkillFrontmatter {
                name: id.to_owned(),
                description: Some(format!("The {id} skill")),
                source: Some(ExternalSkillSource {
                    repo: "owner/repo".to_owned(),
                    path: format!("skills/{id}"),
                    git_ref: "main".to_owned(),
                }),
            },
        )
    }

    fn target(
        id: TargetId,
        skill: &Skill,
        path: &str,
        source_hash: &str,
        status: MaterialStatus,
    ) -> Target {
        Target {
            id,
            skill: skill.id.clone(),
            path: Utf8PathBuf::from(path),
            source_path: skill.dir.clone(),
            source_hash: source_hash.to_owned(),
            status,
        }
    }

    fn provider(mode: RenderMode) -> ProviderEntry {
        ProviderEntry {
            target_path: Utf8PathBuf::from("/target"),
            rendered_hash: "sha256:x".to_owned(),
            linked_at: "2026-01-01T00:00:00.000Z".to_owned(),
            mode: Some(mode),
        }
    }

    fn entry(source_hash: &str, provider: ProviderEntry) -> InstallationEntry {
        InstallationEntry {
            source_path: Utf8PathBuf::from("/source"),
            source_hash: source_hash.to_owned(),
            installed_at: "2026-01-01T00:00:00.000Z".to_owned(),
            commit_sha: None,
            providers: {
                let mut map = HashMap::new();
                map.insert(TargetId::Claude, provider);
                map
            },
        }
    }

    #[test]
    fn create_when_target_missing_and_no_state() {
        let skill = authored_skill("alpha", "/source/alpha");
        let target = target(
            TargetId::Claude,
            &skill,
            "/target/alpha",
            "sha256:any",
            MaterialStatus::Missing,
        );
        let state = State::default();

        let steps = plan(&[skill], &[target], &state);

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].action, Action::Create);
        assert_eq!(steps[0].mode, RenderMode::Symlink);
    }

    #[test]
    fn adopt_existing_matching_target_when_no_state() {
        let skill = authored_skill("alpha", "/source/alpha");
        let target = target(
            TargetId::Claude,
            &skill,
            "/target/alpha",
            "sha256:any",
            MaterialStatus::Ok,
        );
        let state = State::default();

        let steps = plan(&[skill], &[target], &state);

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].action, Action::Create);
        assert_eq!(steps[0].reason.as_deref(), Some("adopt existing target"));
    }

    #[test]
    fn update_when_untracked_obstruction_exists() {
        let skill = authored_skill("alpha", "/source/alpha");
        let target = target(
            TargetId::Claude,
            &skill,
            "/target/alpha",
            "sha256:any",
            MaterialStatus::NotLink,
        );
        let state = State::default();

        let steps = plan(&[skill], &[target], &state);

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].action, Action::Update);
        assert_eq!(
            steps[0].reason.as_deref(),
            Some("untracked target entry exists")
        );
    }

    #[test]
    fn update_when_render_mode_changed() {
        let skill = external_skill("ext", "/source/ext");
        let target = target(
            TargetId::Claude,
            &skill,
            "/target/ext",
            "sha256:any",
            MaterialStatus::Ok,
        );
        let state = State {
            installations: {
                let mut map = HashMap::new();
                map.insert(
                    "ext".to_owned(),
                    entry("sha256:stale", provider(RenderMode::Symlink)),
                );
                map
            },
        };

        let steps = plan(&[skill], &[target], &state);

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].action, Action::Update);
        assert_eq!(steps[0].reason.as_deref(), Some("render mode changed"));
    }

    #[test]
    fn create_when_tracked_target_is_missing() {
        let skill = authored_skill("alpha", "/source/alpha");
        let target = target(
            TargetId::Claude,
            &skill,
            "/target/alpha",
            "sha256:any",
            MaterialStatus::Missing,
        );
        let state = State {
            installations: {
                let mut map = HashMap::new();
                map.insert(
                    "alpha".to_owned(),
                    entry("sha256:stale", provider(RenderMode::Symlink)),
                );
                map
            },
        };

        let steps = plan(&[skill], &[target], &state);

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].action, Action::Create);
    }

    #[test]
    fn update_when_symlink_points_elsewhere() {
        let skill = authored_skill("alpha", "/source/alpha");
        let target = target(
            TargetId::Claude,
            &skill,
            "/target/alpha",
            "sha256:any",
            MaterialStatus::WrongLink,
        );
        let state = State {
            installations: {
                let mut map = HashMap::new();
                map.insert(
                    "alpha".to_owned(),
                    entry("sha256:stale", provider(RenderMode::Symlink)),
                );
                map
            },
        };

        let steps = plan(&[skill], &[target], &state);

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].action, Action::Update);
    }

    #[test]
    fn update_when_tracked_target_is_not_a_symlink() {
        let skill = authored_skill("alpha", "/source/alpha");
        let target = target(
            TargetId::Claude,
            &skill,
            "/target/alpha",
            "sha256:any",
            MaterialStatus::NotLink,
        );
        let state = State {
            installations: {
                let mut map = HashMap::new();
                map.insert(
                    "alpha".to_owned(),
                    entry("sha256:stale", provider(RenderMode::Symlink)),
                );
                map
            },
        };

        let steps = plan(&[skill], &[target], &state);

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].action, Action::Update);
        assert_eq!(steps[0].reason.as_deref(), Some("target is not a symlink"));
    }

    #[test]
    fn unchanged_with_refreshed_hash_when_source_changed() {
        let skill = authored_skill("alpha", "/source/alpha");
        let target = target(
            TargetId::Claude,
            &skill,
            "/target/alpha",
            "sha256:new",
            MaterialStatus::Ok,
        );
        let state = State {
            installations: {
                let mut map = HashMap::new();
                map.insert(
                    "alpha".to_owned(),
                    entry("sha256:old", provider(RenderMode::Symlink)),
                );
                map
            },
        };

        let steps = plan(&[skill], &[target], &state);

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].action, Action::Unchanged);
        assert_eq!(
            steps[0].reason.as_deref(),
            Some("source changed (hash refreshed)")
        );
    }

    #[test]
    fn unchanged_when_everything_matches() {
        let skill = authored_skill("alpha", "/source/alpha");
        let target = target(
            TargetId::Claude,
            &skill,
            "/target/alpha",
            "sha256:same",
            MaterialStatus::Ok,
        );
        let state = State {
            installations: {
                let mut map = HashMap::new();
                map.insert(
                    "alpha".to_owned(),
                    entry("sha256:same", provider(RenderMode::Symlink)),
                );
                map
            },
        };

        let steps = plan(&[skill], &[target], &state);

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].action, Action::Unchanged);
        assert!(steps[0].reason.is_none());
    }

    #[test]
    fn remove_skills_no_longer_declared() {
        let skill = authored_skill("alpha", "/source/alpha");
        let target = target(
            TargetId::Claude,
            &skill,
            "/target/alpha",
            "sha256:any",
            MaterialStatus::Missing,
        );
        let state = State {
            installations: {
                let mut map = HashMap::new();
                map.insert(
                    "ghost".to_owned(),
                    InstallationEntry {
                        source_path: Utf8PathBuf::from("/source/ghost"),
                        source_hash: "sha256:x".to_owned(),
                        installed_at: "2026-01-01T00:00:00.000Z".to_owned(),
                        commit_sha: None,
                        providers: {
                            let mut providers = HashMap::new();
                            providers.insert(
                                TargetId::Claude,
                                ProviderEntry {
                                    target_path: Utf8PathBuf::from("/target/ghost"),
                                    rendered_hash: "sha256:x".to_owned(),
                                    linked_at: "2026-01-01T00:00:00.000Z".to_owned(),
                                    mode: Some(RenderMode::Symlink),
                                },
                            );
                            providers
                        },
                    },
                );
                map
            },
        };

        let steps = plan(&[skill], &[target], &state);

        // Two steps: create the missing alpha, remove the untracked ghost.
        assert_eq!(steps.len(), 2);
        // Steps are sorted by skill name, so ghost comes after alpha.
        assert_eq!(steps[0].action, Action::Create);
        assert_eq!(steps[0].skill.as_str(), "alpha");
        assert_eq!(steps[1].skill.as_str(), "ghost");
        assert_eq!(steps[1].action, Action::Remove);
        assert_eq!(steps[1].target.as_str(), "/target/ghost");
    }

    #[test]
    fn commit_sha_short_circuit_skips() {
        let skill = authored_skill("alpha", "/source/alpha");
        let target = target(
            TargetId::Claude,
            &skill,
            "/target/alpha",
            "sha256:any",
            MaterialStatus::Missing,
        );
        let state = State {
            installations: {
                let mut map = HashMap::new();
                map.insert(
                    "alpha".to_owned(),
                    InstallationEntry {
                        source_path: Utf8PathBuf::from("/source/alpha"),
                        source_hash: "sha256:stale".to_owned(),
                        installed_at: "2026-01-01T00:00:00.000Z".to_owned(),
                        commit_sha: Some("abc123".to_owned()),
                        providers: {
                            let mut providers = HashMap::new();
                            providers.insert(TargetId::Claude, provider(RenderMode::Symlink));
                            providers
                        },
                    },
                );
                map
            },
        };

        let steps = plan_with_options(
            &[skill],
            &[target],
            &state,
            PlanOptions {
                repo_head: Some("abc123".to_owned()),
                tree_clean: true,
            },
        );

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].action, Action::Unchanged);
        assert_eq!(steps[0].reason.as_deref(), Some("commit-sha unchanged"));
    }
}
