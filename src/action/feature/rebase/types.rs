//! What `ivar feature rebase` takes and reports, and its human rendering.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::error::WriteHuman;

/// What a `feature rebase` call does in each selected repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebaseMode {
    /// Start a rebase. A stop is aborted and reported as conflicted, unless
    /// `keep_conflicts` asks for the stopped rebase to be left in progress so
    /// the conflict can be resolved in the worktree.
    Run { keep_conflicts: bool },
    /// `git rebase --continue` in every repo whose rebase was left in progress.
    Continue,
    /// `git rebase --abort` in every repo whose rebase was left in progress.
    Abort,
}

impl Default for RebaseMode {
    fn default() -> Self {
        Self::Run {
            keep_conflicts: false,
        }
    }
}

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
    /// Start a rebase, or finish one that was left in progress.
    pub mode: RebaseMode,
}

/// What happened to one promoted repo's worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RebaseStatus {
    /// The rebase completed — the worktree's branch now sits on its target base's tip.
    Rebased,
    /// The repo was not rebased (dirty worktree, no worktree to rebase, no
    /// rebase in progress to continue, or a rebase that was just aborted).
    Skipped,
    /// The rebase stopped. Unless the repo's `in_progress` is set it was
    /// aborted and the worktree is untouched; with `in_progress` the worktree
    /// is left in the middle of the rebase, awaiting `--continue` or `--abort`.
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
    /// The paths still unmerged in a rebase left in progress; omitted when
    /// there are none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicted_files: Vec<Utf8PathBuf>,
    /// The worktree is still in the middle of a rebase: resolve it, then
    /// `--continue`, or give up with `--abort`. Omitted when false.
    #[serde(skip_serializing_if = "is_false")]
    pub in_progress: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(value: &bool) -> bool {
    !*value
}

impl RepoRebase {
    /// A repo with a status and nothing else to say about it.
    pub(super) fn bare(
        repo: &RepoName,
        status: RebaseStatus,
        onto: Option<BranchName>,
        base_source: Option<BaseSource>,
    ) -> Self {
        Self {
            repo: repo.clone(),
            status,
            onto,
            base_source,
            conflicted_files: Vec::new(),
            in_progress: false,
        }
    }
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
            for path in &repo.conflicted_files {
                writeln!(w, "    conflict: {path}")?;
            }
            if repo.in_progress {
                writeln!(
                    w,
                    "    next: resolve and `git add` the files, then `ivar feature rebase {feature} --continue --repo {repo}`, or give up with `ivar feature rebase {feature} --abort --repo {repo}`",
                    feature = self.feature,
                    repo = repo.repo,
                )?;
            }
        }
        Ok(())
    }
}
