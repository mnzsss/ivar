//! The side-effect-free cleanup preview and its pure eligibility classifier.
//!
//! Actions gather filesystem, git, session, and feature-tree facts, then pass
//! them here. Keeping the decisions here makes cleanup and prune share the one
//! definition of a feature whose local state may be removed.

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use std::fmt;

use super::super::name::{BranchName, FeatureName, RepoName, SessionId};

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
}

impl CleanupRepoFacts {
    #[must_use]
    pub fn to_delivery_facts(&self) -> DeliveryRepoFacts {
        DeliveryRepoFacts {
            repo: self.repo.clone(),
            effective_base: self.effective_base.clone(),
            clone_exists: self.clone_exists,
            unmerged_commits: self.unmerged_commits,
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
    pub descendants: Vec<FeatureName>,
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
    Descendants {
        features: Vec<FeatureName>,
    },
    UnmergedCommits {
        repo: RepoName,
        effective_base: BranchName,
        commits: u64,
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
            CleanupBlocker::Descendants { .. } => write!(f, "has blocking descendants"),
            CleanupBlocker::UnmergedCommits {
                repo,
                effective_base,
                commits,
            } => {
                write!(
                    f,
                    "`{repo}` has {commits} commit(s) not merged into `{effective_base}`"
                )
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

impl CleanupVerdict {
    #[must_use]
    pub fn eligible(&self) -> bool {
        self.blockers.is_empty() || self.blockers == [CleanupBlocker::EmptyFeature]
    }
}

/// Classify a feature for cleanup (combining delivery + operational checks).
#[must_use]
pub fn classify_cleanup(facts: &CleanupFacts) -> CleanupVerdict {
    let mut blockers = Vec::new();

    let delivery_facts = facts.to_delivery_facts();
    if let DeliveryVerdict::Blocked(delivery_blockers) = classify_delivery(&delivery_facts) {
        for b in delivery_blockers {
            blockers.push(CleanupBlocker::from(b));
        }
    }

    if !facts.descendants.is_empty() {
        blockers.push(CleanupBlocker::Descendants {
            features: facts.descendants.clone(),
        });
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
}

pub const CLEANUP_RECORD_SCHEMA_VERSION: u32 = 1;

/// The documentation approval decision in a cleanup record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentationDecision {
    Written,
    NotRequired,
}

impl fmt::Display for DocumentationDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocumentationDecision::Written => write!(f, "written"),
            DocumentationDecision::NotRequired => write!(f, "not_required"),
        }
    }
}

/// Documentation approval details in a cleanup record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentationApproval {
    pub decision: DocumentationDecision,
    pub paths: Vec<Utf8PathBuf>,
    pub reason: Option<String>,
    pub at: String,
}

/// Delivery approval details in a cleanup record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryApproval {
    pub approved: bool,
    pub at: String,
}

/// Teardown approval details in a cleanup record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeardownApproval {
    pub approved: bool,
    pub at: String,
}

/// The three approval gates recorded in a cleanup record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupApprovals {
    pub delivery: DeliveryApproval,
    pub documentation: DocumentationApproval,
    pub teardown: TeardownApproval,
}

/// One promoted repo's worktree removal outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRemoval {
    pub repo: RepoName,
    pub removed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One promoted repo's local branch deletion outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BranchDeletion {
    pub repo: RepoName,
    pub deleted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Structured outcome of applying cleanup teardown, stored in the cleanup record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupApplyOutcome {
    pub feature: FeatureName,
    pub branch: BranchName,
    pub fingerprint: String,
    pub worktrees: Vec<WorktreeRemoval>,
    pub branches: Vec<BranchDeletion>,
    pub feature_removed: bool,
    pub plans_removed: bool,
}

/// The durable cleanup record schema version 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupRecord {
    pub schema_version: u32,
    pub feature: FeatureName,
    pub branch: BranchName,
    pub fingerprint: String,
    pub approvals: CleanupApprovals,
    pub outcome: Option<CleanupApplyOutcome>,
}

impl CleanupRecord {
    /// Validate intrinsic field rules of a cleanup record.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != CLEANUP_RECORD_SCHEMA_VERSION {
            return Err(format!(
                "unsupported schema_version `{}` (expected {})",
                self.schema_version, CLEANUP_RECORD_SCHEMA_VERSION
            ));
        }

        if self.outcome.is_some() {
            return Err("cleanup record outcome is already populated".to_owned());
        }

        let doc = &self.approvals.documentation;
        match doc.decision {
            DocumentationDecision::Written => {
                if doc.paths.is_empty() {
                    return Err(
                        "documentation decision `written` requires non-empty `paths`".to_owned(),
                    );
                }
                if doc.reason.is_some() {
                    return Err(
                        "documentation decision `written` requires null `reason`".to_owned()
                    );
                }
            }
            DocumentationDecision::NotRequired => {
                if !doc.paths.is_empty() {
                    return Err(
                        "documentation decision `not_required` requires empty `paths`".to_owned(),
                    );
                }
                match &doc.reason {
                    Some(reason) if !reason.trim().is_empty() => {}
                    _ => {
                        return Err(
                            "documentation decision `not_required` requires a non-empty `reason`"
                                .to_owned(),
                        );
                    }
                }
            }
        }

        for path in &doc.paths {
            if path.is_absolute()
                || !path.starts_with("docs")
                || path.components().any(|c| c.as_str() == "..")
            {
                return Err(format!(
                    "documentation path `{path}` is not hall-relative or does not resolve inside `docs/`"
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/feature/cleanup.rs"]
mod tests;
