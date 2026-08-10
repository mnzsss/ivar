//! `ivar repo list` — show every repo the hall knows about, and its state.
//!
//! Read-only. It looks at what `ivar.json` declares and what exists under
//! `.ivar/`, and never mutates either.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::error::{Outcome, Report, WriteHuman};
use crate::git::{self, TargetState};
use crate::store::layout::Layout;
use crate::store::manifest::Repo;

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// One repo's observed state.
#[derive(Debug, Clone, Serialize)]
pub struct RepoStatus {
    /// The repo's name, as declared in `ivar.json`.
    pub name: RepoName,
    /// The git remote URL.
    pub url: String,
    /// The branch a fresh worktree defaults to.
    pub default_branch: String,
    /// Whether the bare clone exists under `.ivar/`.
    pub bare_cloned: bool,
    /// Whether the default-branch worktree exists.
    pub default_worktree: bool,
    /// Every branch the bare clone knows about, sorted.
    pub branches: Vec<String>,
}

/// What `ivar repo list` found.
#[derive(Debug, Clone, Serialize)]
pub struct ListOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// One entry per repo in `ivar.json`, in manifest order.
    pub repos: Vec<RepoStatus>,
}

impl WriteHuman for ListOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.repos.is_empty() {
            writeln!(w, "No repos in {}.", self.root)?;
            return Ok(());
        }
        writeln!(w, "Repos in {}:", self.root)?;
        for repo in &self.repos {
            let bare = if repo.bare_cloned {
                "cloned"
            } else {
                "missing"
            };
            let worktree = if repo.default_worktree {
                String::new()
            } else {
                " (no worktree)".to_owned()
            };
            let branches = if repo.branches.is_empty() {
                String::new()
            } else {
                format!("  [{}]", repo.branches.join(", "))
            };
            writeln!(
                w,
                "  {}  {bare}  {}{worktree}  ← {}{branches}",
                repo.name, repo.default_branch, repo.url,
            )?;
        }
        Ok(())
    }
}

/// List every repo declared in `ivar.json`, with its on-disk state.
///
/// A repo whose bare clone cannot be read (corrupt, or gone mid-listing)
/// reports `bare_cloned: false` and an empty branch list rather than failing
/// the whole listing — this is a status command, and one broken repo should
/// not hide the other seven.
pub fn list(ctx: &Ctx) -> Outcome<ListOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let repos = manifest
        .repos()
        .iter()
        .map(|repo| status_of(&git, &layout, repo))
        .collect();

    Ok(Report::new(ListOutcome {
        root: layout.root().to_path_buf(),
        repos,
    }))
}

/// Observe one repo's on-disk state without letting any single probe fail
/// the listing.
fn status_of(git: &impl git::Git, layout: &Layout, repo: &Repo) -> RepoStatus {
    let bare = layout.repo_bare(repo.name());
    let worktree = layout.repo_worktree(repo.name(), repo.default_branch());

    let bare_state = git.target_state(&bare).unwrap_or(TargetState::Absent);
    let worktree_state = git.target_state(&worktree).unwrap_or(TargetState::Absent);

    let branches = if matches!(bare_state, TargetState::Repository) {
        git.list_branches(&bare).unwrap_or_default()
    } else {
        Vec::new()
    };

    RepoStatus {
        name: repo.name().clone(),
        url: repo.url().to_owned(),
        default_branch: repo.default_branch().to_string(),
        bare_cloned: matches!(bare_state, TargetState::Repository),
        default_worktree: matches!(worktree_state, TargetState::Repository),
        branches,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/repo/list.rs"]
mod tests;
