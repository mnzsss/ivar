use crate::domain::name::{BranchName, FeatureName, RepoName};
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use std::fmt;

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
///
/// Note: `deny_unknown_fields` is omitted intentionally so older records written
/// by ivar <= 0.8 that contain `"plans_removed": true` can be deserialised
/// without backwards-compatibility errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupApplyOutcome {
    pub feature: FeatureName,
    pub branch: BranchName,
    pub fingerprint: String,
    pub worktrees: Vec<WorktreeRemoval>,
    pub branches: Vec<BranchDeletion>,
    pub feature_removed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub descendants: Vec<CleanupDescendantOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupDescendantOutcome {
    pub feature: FeatureName,
    pub branch: BranchName,
    pub worktrees: Vec<WorktreeRemoval>,
    pub branches: Vec<BranchDeletion>,
    pub feature_removed: bool,
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
    ///
    /// # Errors
    ///
    /// Returns a message if `schema_version` does not match the current
    /// schema, the outcome is already populated, or the documentation
    /// decision's fields violate its own invariants (e.g. `written` without
    /// `paths`, or with a `reason` set).
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
                || !(path.starts_with("docs/memory") || path.starts_with("docs/repo-relations"))
                || path.components().any(|c| c.as_str() == "..")
            {
                return Err(format!(
                    "documentation path `{path}` is not hall-relative or does not resolve inside `docs/memory/` or `docs/repo-relations/`"
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/domain/feature/cleanup_approvals.rs"]
mod tests;
