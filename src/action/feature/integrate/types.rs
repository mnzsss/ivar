use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{FeatureIntegrationState, IntegrationPolicy};
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::error::WriteHuman;

/// What `ivar feature integrate` needs.
#[derive(Debug, Clone)]
pub struct IntegrateInput {
    /// The child feature to integrate into its immediate parent.
    pub feature: String,
    /// A via override — `pr` or `local`, unvalidated.
    pub via: Option<String>,
    /// A strategy override — `squash`, `merge`, or `rebase`, unvalidated.
    pub strategy: Option<String>,
    /// The integration title — the squash or merge commit message on the
    /// parent, and with `--via pr` the PR title and merge subject. Defaults
    /// to `feat: integrate <child>`.
    pub name: Option<String>,
}

/// One repo's integration result within a run.
#[derive(Debug, Clone, Serialize)]
pub struct RepoIntegration {
    /// The repo.
    pub repo: RepoName,
    /// The child branch's tip this repo was integrated at.
    pub source_sha: String,
    /// The immediate parent's branch — the only target a child ever has.
    pub target_branch: BranchName,
    /// The result commit on the parent's branch, once applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_sha: Option<String>,
    /// What happened to this repo this run.
    pub status: RepoIntegrationStatus,
    /// The pull request that carried the change, when `via=pr`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    /// Why this repo is pending, failed, or stale.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// What happened to one repo this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoIntegrationStatus {
    /// A fresh passing receipt was validated and reused; nothing moved.
    Reused,
    /// The repo was integrated now.
    Integrated,
    /// Waiting on something resumable (a pending PR check, an observe
    /// timeout).
    Pending,
    /// The integration failed — failed evidence, or a refused merge.
    Failed,
    /// The receipt no longer matches live state.
    Stale,
}

/// What `ivar feature integrate` did.
#[derive(Debug, Clone, Serialize)]
pub struct IntegrateOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The child that was integrated.
    pub feature: FeatureName,
    /// The immediate parent it integrated into.
    pub parent: FeatureName,
    /// The resolved integration policy for this run.
    pub policy: IntegrationPolicy,
    /// One entry per promoted repo, in name order.
    pub repos: Vec<RepoIntegration>,
    /// The child's derived integration state after the run.
    pub state: FeatureIntegrationState,
    /// Whether the run closed the child with outcome `integrated`.
    pub closed_integrated: bool,
}

impl std::fmt::Display for RepoIntegrationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Reused => "reused",
            Self::Integrated => "integrated",
            Self::Pending => "pending",
            Self::Failed => "failed",
            Self::Stale => "stale",
        };
        f.pad(name)
    }
}

impl std::fmt::Display for IntegrationPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.via, self.strategy)
    }
}

impl WriteHuman for IntegrateOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Integrated `{}` into `{}` ({}):",
            self.feature, self.parent, self.policy
        )?;
        for repo in &self.repos {
            let detail = repo
                .detail
                .as_deref()
                .map(|d| format!(" — {d}"))
                .unwrap_or_default();
            let result = repo
                .result_sha
                .as_deref()
                .map(|sha| format!(" at {sha}"))
                .unwrap_or_default();
            writeln!(w, "  {}  {}{}{detail}", repo.repo, repo.status, result)?;
        }
        if self.closed_integrated {
            writeln!(
                w,
                "Closed `{}` as integrated; the outcome is final.",
                self.feature
            )?;
        }
        Ok(())
    }
}
