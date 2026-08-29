//! The session write guard: determines which files a session may write.

use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::feature::Feature;
use crate::error::Failure;
use crate::store::layout::Layout;

/// The set of paths a session is allowed to write into: its view dir plus
/// the worktrees of promoted repos.
#[derive(Debug, Clone)]
pub(crate) struct WritableSet {
    view_dir: Utf8PathBuf,
    worktrees: Vec<Utf8PathBuf>,
}

impl WritableSet {
    /// Build the writable set from the session's view dir and the feature's
    /// promoted repos.
    pub fn from_session(
        layout: &Layout,
        feature: &Feature,
        view_dir: &Utf8Path,
    ) -> Result<Self, Failure> {
        let worktrees = feature
            .promotions
            .keys()
            .map(|repo| layout.repo_worktree(repo, &feature.branch))
            .collect();
        Ok(Self {
            view_dir: view_dir.to_path_buf(),
            worktrees,
        })
    }

    /// Whether `path` is inside the view dir or one of the promoted worktrees.
    pub(crate) fn allows(&self, path: &Utf8Path) -> bool {
        if path.starts_with(&self.view_dir) {
            return true;
        }
        self.worktrees.iter().any(|wt| path.starts_with(wt))
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/guard.rs"]
mod tests;
