//! What `ivar feature rebase` takes and reports, and its human rendering.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::error::WriteHuman;

/// What `ivar feature rebase` needs.
#[derive(Debug, Clone)]
pub struct RebaseInput {
    /// The feature's name.
    pub name: String,
    /// Collapse the base: rebase every selected repo onto this branch,
    /// unvalidated, and record it as the declared base for each repo that
    /// actually lands there.
    pub onto: Option<String>,
    /// Promoted repos to rebase; empty means every promoted repo.
    pub repos: Vec<String>,
    /// Rebase onto the local base ref and make no network call.
    pub offline: bool,
}

/// What happened to one promoted repo's worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RebaseStatus {
    /// The rebase completed — the worktree's branch now sits on its target base's tip.
    Rebased,
    /// The repo was not rebased (dirty worktree, or no worktree to rebase).
    Skipped,
    /// The rebase stopped and was aborted; the worktree is untouched.
    Conflicted,
}

/// Where the tip a repo was rebased onto came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseSource {
    /// The remote's tip, fetched into the worktree's `FETCH_HEAD`.
    Remote,
    /// The hall's local base ref.
    Local,
}

/// One promoted repo's rebase result.
#[derive(Debug, Clone, Serialize)]
pub struct RepoRebase {
    pub repo: RepoName,
    pub status: RebaseStatus,
    /// The base this repo was rebased (or tried to rebase) onto; `None` only
    /// when the repo is not in `ivar.json`.
    pub onto: Option<BranchName>,
    /// Where `onto`'s tip came from; `None` when the repo was skipped
    /// before any rebase ran.
    pub base_source: Option<BaseSource>,
}

/// What `ivar feature rebase` did.
#[derive(Debug, Clone, Serialize)]
pub struct RebaseOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature whose repos were rebased.
    pub feature: FeatureName,
    /// The feature branch every promoted worktree is on.
    pub branch: String,
    /// One entry per selected promoted repo, in name order.
    pub repos: Vec<RepoRebase>,
}

impl WriteHuman for RebaseOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Rebased feature `{}` (branch: {}) in {}:",
            self.feature, self.branch, self.root
        )?;
        if self.repos.is_empty() {
            writeln!(w, "  no repos promoted")?;
        }
        for repo in &self.repos {
            let status = match repo.status {
                RebaseStatus::Rebased => "rebased",
                RebaseStatus::Skipped => "skipped",
                RebaseStatus::Conflicted => "conflicted",
            };
            match (&repo.onto, repo.base_source) {
                (Some(onto), Some(source)) => writeln!(
                    w,
                    "  {}  {status}  onto {onto} ({})",
                    repo.repo,
                    match source {
                        BaseSource::Remote => "remote",
                        BaseSource::Local => "local",
                    }
                )?,
                _ => writeln!(w, "  {}  {status}", repo.repo)?,
            }
        }
        Ok(())
    }
}
