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
    pub(crate) fn from_session(
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

    /// The view dir — the canonical root of this set.
    pub(crate) fn view_dir(&self) -> &Utf8Path {
        &self.view_dir
    }

    /// Build a `WritableSet` from explicit parts. Test-only.
    #[cfg(test)]
    pub(crate) fn from_parts(view_dir: Utf8PathBuf, worktrees: Vec<Utf8PathBuf>) -> Self {
        Self { view_dir, worktrees }
    }
}

/// A tool invocation the guard is asked to evaluate.
#[derive(Debug)]
pub(crate) struct ToolRequest {
    pub tool: String,
    pub file_path: Option<Utf8PathBuf>,
}

/// The guard's decision for a tool request.
#[derive(Debug)]
pub(crate) enum GuardDecision {
    Allow,
    Deny { reason: String },
}

/// Decide whether a tool request is allowed inside the session.
///
/// Write and Edit tools are checked against the writable set; everything
/// else is allowed.
pub(crate) fn decide(set: Option<&WritableSet>, req: &ToolRequest) -> GuardDecision {
    let tool_lower = req.tool.to_ascii_lowercase();
    match tool_lower.as_str() {
        "write" | "edit" => {
            let Some(set) = set else {
                return GuardDecision::Deny {
                    reason: "no ivar session resolves from the cwd".into(),
                };
            };
            match &req.file_path {
                Some(path) if set.allows(path) => GuardDecision::Allow,
                _ => {
                    let members: Vec<String> = std::iter::once(set.view_dir().to_string())
                        .chain(set.worktrees.iter().map(|w| w.to_string()))
                        .collect();
                    GuardDecision::Deny {
                        reason: format!(
                            "writable set: {}",
                            members.join(", ")
                        ),
                    }
                }
            }
        }
        _ => GuardDecision::Allow,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/guard.rs"]
mod tests;
