//! Land mode execution: preflight validation, permission guard, fast-forward merge, and best-effort push.

pub mod execute;
pub mod permissions;
pub mod preflight;

pub use execute::execute;
pub use permissions::WorktreeWriteGuard;
pub use preflight::preflight;

use camino::Utf8PathBuf;

use crate::domain::name::{BranchName, RepoName};

/// What land mode executes per repository (ADR-0004).
#[derive(Debug, Clone)]
pub struct LandPlan {
    pub repo: RepoName,
    pub worktree: Utf8PathBuf,
    pub default_branch: BranchName,
    pub tip: String,
    pub remote: String,
    pub remote_default_tip: Option<String>,
    pub original_head: String,
    pub feature_name: String,
}
