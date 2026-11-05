//! The side-effect-free cleanup preview and its pure eligibility classifier.
//!
//! Actions gather filesystem, git, session, and feature-tree facts, then pass
//! them here. Keeping the decisions here makes cleanup and prune share the one
//! definition of a feature whose local state may be removed.

use super::super::name::{BranchName, FeatureName, RepoName, SessionId};
use super::integration::FeatureIntegrationState;
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use std::fmt;
/// Minimal git and manifest facts for one promoted repository to judge delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryRepoFacts {
    pub repo: RepoName,
    pub effective_base: Option<BranchName>,
    pub clone_exists: bool,
    pub unmerged_commits: Option<u64>,
    pub in_manifest: bool,
    pub inspection_error: Option<String>,
}

/// Facts needed by prune and cleanup to judge feature delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryFacts {
    pub repos: Vec<DeliveryRepoFacts>,
    pub live_sessions: Vec<SessionId>,
    pub session_inspection_error: Option<String>,
}

/// A machine-readable reason why a feature is not delivered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeliveryBlocker {
    LiveSessions {
        sessions: Vec<SessionId>,
    },
    SessionInspectionFailed {
        error: String,
    },
    RepoAbsentFromManifest {
        repo: RepoName,
    },
    MissingClone {
        repo: RepoName,
    },
    UnmergedCommits {
        repo: RepoName,
        effective_base: BranchName,
        commits: u64,
    },
    RepositoryInspectionFailed {
        repo: RepoName,
        error: String,
    },
}

impl fmt::Display for DeliveryBlocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeliveryBlocker::LiveSessions { .. } => write!(f, "has a live session"),
            DeliveryBlocker::SessionInspectionFailed { error } => {
                write!(f, "cannot check its sessions: {error}")
            }
            DeliveryBlocker::RepoAbsentFromManifest { repo } => {
                write!(f, "repo `{repo}` is no longer in ivar.json")
            }
            DeliveryBlocker::MissingClone { repo } => {
                write!(
                    f,
                    "cannot check `{repo}` — its clone is missing (run `ivar sync`)"
                )
            }
            DeliveryBlocker::UnmergedCommits {
                repo,
                effective_base,
                commits,
            } => {
                write!(
                    f,
                    "`{repo}` has {commits} commit(s) not merged into `{effective_base}`"
                )
            }
            DeliveryBlocker::RepositoryInspectionFailed { repo, error } => {
                write!(f, "cannot check `{repo}`: {error}")
            }
        }
    }
}

/// The pure delivery decision over already-gathered facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryVerdict {
    Delivered,
    Blocked(Vec<DeliveryBlocker>),
}

impl DeliveryVerdict {
    #[must_use]
    pub fn is_delivered(&self) -> bool {
        matches!(self, DeliveryVerdict::Delivered)
    }
}

/// Classify feature delivery without reading from the filesystem or invoking git.
#[must_use]
pub fn classify_delivery(facts: &DeliveryFacts) -> DeliveryVerdict {
    let mut blockers = Vec::new();

    if !facts.live_sessions.is_empty() {
        blockers.push(DeliveryBlocker::LiveSessions {
            sessions: facts.live_sessions.clone(),
        });
    }
    if let Some(error) = &facts.session_inspection_error {
        blockers.push(DeliveryBlocker::SessionInspectionFailed {
            error: error.clone(),
        });
    }

    for repo in &facts.repos {
        if !repo.in_manifest {
            blockers.push(DeliveryBlocker::RepoAbsentFromManifest {
                repo: repo.repo.clone(),
            });
        }
        if !repo.clone_exists {
            blockers.push(DeliveryBlocker::MissingClone {
                repo: repo.repo.clone(),
            });
        }
        if let (Some(commits @ 1..), Some(effective_base)) =
            (repo.unmerged_commits, repo.effective_base.as_ref())
        {
            blockers.push(DeliveryBlocker::UnmergedCommits {
                repo: repo.repo.clone(),
                effective_base: effective_base.clone(),
                commits,
            });
        }
        if let Some(error) = &repo.inspection_error {
            blockers.push(DeliveryBlocker::RepositoryInspectionFailed {
                repo: repo.repo.clone(),
                error: error.clone(),
            });
        }
    }

    if blockers.is_empty() {
        DeliveryVerdict::Delivered
    } else {
        DeliveryVerdict::Blocked(blockers)
    }
}

/// What the forge says about a branch local git could not prove delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForgeDelivery {
    /// A merged pull request whose head is the local feature head.
    Merged,
    /// The forge answered, and lists no pull request for the branch at all.
    NoPullRequest,
    /// A merged pull request exists, but it landed a different head.
    MergedOtherHead { number: u64 },
    /// A pull request exists and has not merged.
    NotMerged { number: u64, state: String },
    /// The forge could not answer.
    Unavailable { reason: String },
}

impl ForgeDelivery {
    /// The sentence a blocker carries for this verdict, or `None` when the
    /// verdict removes the blocker instead of explaining it.
    fn detail(&self) -> Option<String> {
        match self {
            Self::Merged => None,
            Self::NoPullRequest => {
                Some("the forge lists no pull request for this branch".to_owned())
            }
            Self::MergedOtherHead { number } => Some(format!(
                "pull request #{number} merged a head other than this branch's"
            )),
            Self::NotMerged { number, state } => Some(format!("pull request #{number} is {state}")),
            Self::Unavailable { reason } => Some(reason.clone()),
        }
    }
}

/// Git and filesystem facts for one promoted repository, gathered by an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupRepoFacts {
    pub repo: RepoName,
    pub effective_base: Option<BranchName>,
    pub feature_head: Option<String>,
    pub base_head: Option<String>,
    pub local_branch_exists: bool,
    pub worktree_exists: bool,
    pub clone_exists: bool,
    pub dirty_worktree: Option<bool>,
    pub unmerged_commits: Option<u64>,
    pub in_manifest: bool,
    pub inspection_error: Option<String>,
    pub forge_delivery: Option<ForgeDelivery>,
}

impl CleanupRepoFacts {
    #[must_use]
    pub fn to_delivery_facts(&self) -> DeliveryRepoFacts {
        let unmerged_commits = if self.forge_delivery == Some(ForgeDelivery::Merged) {
            Some(0)
        } else {
            self.unmerged_commits
        };
        DeliveryRepoFacts {
            repo: self.repo.clone(),
            effective_base: self.effective_base.clone(),
            clone_exists: self.clone_exists,
            unmerged_commits,
            in_manifest: self.in_manifest,
            inspection_error: self.inspection_error.clone(),
        }
    }
}

/// Facts shared by cleanup and prune.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupFacts {
    pub repos: Vec<CleanupRepoFacts>,
    pub live_sessions: Vec<SessionId>,
    pub session_inspection_error: Option<String>,
}
impl CleanupFacts {
    #[must_use]
    pub fn to_delivery_facts(&self) -> DeliveryFacts {
        DeliveryFacts {
            repos: self
                .repos
                .iter()
                .map(CleanupRepoFacts::to_delivery_facts)
                .collect(),
            live_sessions: self.live_sessions.clone(),
            session_inspection_error: self.session_inspection_error.clone(),
        }
    }
}

/// A machine-readable reason cleanup cannot proceed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CleanupBlocker {
    LiveSessions {
        sessions: Vec<SessionId>,
    },
    NotIntegrated {
        state: FeatureIntegrationState,
    },
    UnmergedCommits {
        repo: RepoName,
        effective_base: BranchName,
        commits: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        forge: Option<String>,
    },
    DirtyWorktree {
        repo: RepoName,
    },
    MissingWorktree {
        repo: RepoName,
    },
    MissingClone {
        repo: RepoName,
    },
    RepoAbsentFromManifest {
        repo: RepoName,
    },
    EmptyFeature,
    SessionInspectionFailed {
        error: String,
    },
    RepositoryInspectionFailed {
        repo: RepoName,
        error: String,
    },
}

impl From<DeliveryBlocker> for CleanupBlocker {
    fn from(b: DeliveryBlocker) -> Self {
        match b {
            DeliveryBlocker::LiveSessions { sessions } => CleanupBlocker::LiveSessions { sessions },
            DeliveryBlocker::SessionInspectionFailed { error } => {
                CleanupBlocker::SessionInspectionFailed { error }
            }
            DeliveryBlocker::RepoAbsentFromManifest { repo } => {
                CleanupBlocker::RepoAbsentFromManifest { repo }
            }
            DeliveryBlocker::MissingClone { repo } => CleanupBlocker::MissingClone { repo },
            DeliveryBlocker::UnmergedCommits {
                repo,
                effective_base,
                commits,
            } => CleanupBlocker::UnmergedCommits {
                repo,
                effective_base,
                commits,
                forge: None,
            },
            DeliveryBlocker::RepositoryInspectionFailed { repo, error } => {
                CleanupBlocker::RepositoryInspectionFailed { repo, error }
            }
        }
    }
}

impl fmt::Display for CleanupBlocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CleanupBlocker::LiveSessions { .. } => write!(f, "has a live session"),
            CleanupBlocker::NotIntegrated { state } => {
                write!(f, "subfeature is `{state}`; integrate or abandon it first")
            }
            CleanupBlocker::UnmergedCommits {
                repo,
                effective_base,
                commits,
                forge,
            } => {
                write!(
                    f,
                    "`{repo}` has {commits} commit(s) not merged into `{effective_base}`"
                )?;
                match forge {
                    Some(detail) => write!(f, " (forge: {detail})"),
                    None => Ok(()),
                }
            }
            CleanupBlocker::DirtyWorktree { repo } => {
                write!(f, "cannot check `{repo}` — it has uncommitted changes")
            }
            CleanupBlocker::MissingWorktree { repo } => write!(
                f,
                "cannot check `{repo}` — its worktree is missing (run `ivar sync`)"
            ),
            CleanupBlocker::MissingClone { repo } => write!(
                f,
                "cannot check `{repo}` — its clone is missing (run `ivar sync`)"
            ),
            CleanupBlocker::RepoAbsentFromManifest { repo } => {
                write!(f, "repo `{repo}` is no longer in ivar.json")
            }
            CleanupBlocker::EmptyFeature => write!(f, "feature is empty"),
            CleanupBlocker::SessionInspectionFailed { error } => {
                write!(f, "cannot check its sessions: {error}")
            }
            CleanupBlocker::RepositoryInspectionFailed { repo, error } => {
                write!(f, "cannot check `{repo}`: {error}")
            }
        }
    }
}

/// The pure cleanup decision over already-gathered facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupVerdict {
    pub blockers: Vec<CleanupBlocker>,
}

/// Classify a feature for cleanup (combining delivery + operational checks).
#[must_use]
pub fn classify_cleanup(facts: &CleanupFacts) -> CleanupVerdict {
    let mut blockers = Vec::new();

    let delivery_facts = facts.to_delivery_facts();
    if let DeliveryVerdict::Blocked(delivery_blockers) = classify_delivery(&delivery_facts) {
        for delivery_blocker in delivery_blockers {
            let mut blocker = CleanupBlocker::from(delivery_blocker);
            if let CleanupBlocker::UnmergedCommits { repo, forge, .. } = &mut blocker {
                *forge = facts
                    .repos
                    .iter()
                    .find(|facts| &facts.repo == repo)
                    .and_then(|facts| facts.forge_delivery.as_ref())
                    .and_then(ForgeDelivery::detail);
            }
            blockers.push(blocker);
        }
    }

    if facts.repos.is_empty() {
        blockers.push(CleanupBlocker::EmptyFeature);
    }

    for repo in &facts.repos {
        if !repo.worktree_exists {
            blockers.push(CleanupBlocker::MissingWorktree {
                repo: repo.repo.clone(),
            });
        }
        if repo.dirty_worktree == Some(true) {
            blockers.push(CleanupBlocker::DirtyWorktree {
                repo: repo.repo.clone(),
            });
        }
    }

    CleanupVerdict { blockers }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupDescendantFacts {
    pub state: FeatureIntegrationState,
    pub repos: Vec<CleanupRepoFacts>,
    pub live_sessions: Vec<SessionId>,
    pub session_inspection_error: Option<String>,
}

#[must_use]
pub fn classify_descendant_cleanup(facts: &CleanupDescendantFacts) -> CleanupVerdict {
    let mut blockers = Vec::new();
    match facts.state {
        FeatureIntegrationState::Integrated | FeatureIntegrationState::Abandoned => {}
        FeatureIntegrationState::Active => {
            blockers.push(CleanupBlocker::NotIntegrated {
                state: FeatureIntegrationState::Active,
            });
        }
        FeatureIntegrationState::Failed => {
            blockers.push(CleanupBlocker::NotIntegrated {
                state: FeatureIntegrationState::Failed,
            });
        }
        FeatureIntegrationState::Stale => {
            blockers.push(CleanupBlocker::NotIntegrated {
                state: FeatureIntegrationState::Stale,
            });
        }
        FeatureIntegrationState::Delivered => {
            blockers.push(CleanupBlocker::NotIntegrated {
                state: FeatureIntegrationState::Delivered,
            });
        }
    }
    if !facts.live_sessions.is_empty() {
        blockers.push(CleanupBlocker::LiveSessions {
            sessions: facts.live_sessions.clone(),
        });
    }
    if let Some(error) = &facts.session_inspection_error {
        blockers.push(CleanupBlocker::SessionInspectionFailed {
            error: error.clone(),
        });
    }
    for repo in &facts.repos {
        if repo.dirty_worktree == Some(true) {
            blockers.push(CleanupBlocker::DirtyWorktree {
                repo: repo.repo.clone(),
            });
        }
        if let Some(error) = &repo.inspection_error {
            blockers.push(CleanupBlocker::RepositoryInspectionFailed {
                repo: repo.repo.clone(),
                error: error.clone(),
            });
        }
    }
    CleanupVerdict { blockers }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupDescendant {
    pub feature: FeatureName,
    pub branch: BranchName,
    pub parent: FeatureName,
    pub depth: usize,
    pub state: FeatureIntegrationState,
    pub repos: Vec<CleanupRepo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<CleanupBlocker>,
    pub paths_to_remove: Vec<Utf8PathBuf>,
}

/// One promoted repository's minimal cleanup evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupRepo {
    pub repo: RepoName,
    pub effective_base: BranchName,
    pub feature_head: Option<String>,
    pub base_head: Option<String>,
    pub local_branch_exists: bool,
    pub worktree_exists: bool,
    pub is_delivered: bool,
}

/// The serializable, side-effect-free cleanup summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupPreview {
    pub feature: FeatureName,
    pub branch: BranchName,
    pub repos: Vec<CleanupRepo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<CleanupBlocker>,
    pub paths_to_remove: Vec<Utf8PathBuf>,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub descendants: Vec<CleanupDescendant>,
}

impl CleanupPreview {
    #[must_use]
    pub fn has_blockers(&self) -> bool {
        !self.blockers.is_empty()
            || self
                .descendants
                .iter()
                .any(|descendant| !descendant.blockers.is_empty())
    }
}

mod approvals;
pub use approvals::*;

#[cfg(test)]
#[path = "../../../tests/unit/domain/feature/cleanup.rs"]
mod tests;
