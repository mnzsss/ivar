//! `ivar status` — hall health, derived and rendered.

// ---------------------------------------------------------------------------
// `ivar status` — hall health, derived and rendered.
// ---------------------------------------------------------------------------

/// What `ivar status` found.
use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::health::{Health, RepoHealth};
use crate::error::{Outcome, Report, WriteHuman};
use crate::git::{self, Git, TargetState};

use super::Ctx;
use super::{discover_hall, read_manifest};

#[derive(Debug, Clone, Serialize)]
pub struct StatusOutcome {
    /// The hall root.
    pub root: Utf8PathBuf,
    /// The hall's overall health.
    pub health: &'static str,
    /// One entry per repo, with its observed state.
    pub repos: Vec<RepoStatusEntry>,
}

/// One repo's observed state for the status report.
#[derive(Debug, Clone, Serialize)]
pub struct RepoStatusEntry {
    /// The repo's name.
    pub name: crate::domain::name::RepoName,
    /// Whether the bare clone exists.
    pub bare_cloned: bool,
    /// Whether the default worktree exists.
    pub worktree: bool,
}

impl WriteHuman for StatusOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Hall at {} — {}", self.root, self.health)?;
        for repo in &self.repos {
            let bare = if repo.bare_cloned {
                "cloned"
            } else {
                "missing"
            };
            let worktree = if repo.worktree {
                "worktree ok"
            } else {
                "no worktree"
            };
            writeln!(w, "  {}  {bare}  {worktree}", repo.name)?;
        }
        Ok(())
    }
}

/// Report the hall's health. Read-only — never mutates anything.
pub fn status(ctx: &Ctx) -> Outcome<StatusOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let mut repos = Vec::new();
    for repo in manifest.repos() {
        let bare = layout.repo_bare(repo.name());
        let worktree = layout.repo_worktree(repo.name(), repo.default_branch());
        let bare_state = git.target_state(&bare)?;
        let bare_cloned = matches!(bare_state, TargetState::Repository);
        let worktree_state = if bare_cloned {
            git.target_state(&worktree)?
        } else {
            TargetState::Absent
        };
        repos.push(RepoStatusEntry {
            name: repo.name().clone(),
            bare_cloned,
            worktree: matches!(worktree_state, TargetState::Repository),
        });
    }

    let health = Health::derive(
        &repos
            .iter()
            .map(|repo| RepoHealth {
                bare_cloned: repo.bare_cloned,
                default_worktree_present: Some(repo.worktree),
                ahead_of_bare: false,
            })
            .collect::<Vec<_>>(),
    );

    Ok(Report::new(StatusOutcome {
        root: layout.root().to_path_buf(),
        health: health_word(health),
        repos,
    }))
}

/// The one-word health label for the report. The ladder lives in
/// `domain::health`; this is only the rendering.
fn health_word(health: Health) -> &'static str {
    match health {
        Health::Uninitialized => "uninitialized",
        Health::Operational => "operational",
        Health::Stale => "stale",
        Health::Degraded => "degraded",
    }
}
