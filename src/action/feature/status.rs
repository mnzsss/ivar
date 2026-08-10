//! `ivar feature status <feature>` — one feature in detail: every promoted
//! repo and how far its promotion got.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{Feature, WorktreeState};
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::git::{self, Git, TargetState};

use super::super::discover_hall;
use crate::action::Ctx;

/// One promoted repo's status within a feature.
#[derive(Debug, Clone, Serialize)]
pub struct RepoDetail {
    /// The repo's name.
    pub repo: RepoName,
    /// The recorded worktree state.
    pub state: WorktreeState,
    /// Whether the worktree actually exists on disk right now.
    pub worktree_present: bool,
}

/// What `ivar feature status` found.
#[derive(Debug, Clone, Serialize)]
pub struct StatusOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature.
    pub name: FeatureName,
    /// The branch every promoted repo's worktree is on.
    pub branch: String,
    /// One entry per promoted repo, in name order.
    pub repos: Vec<RepoDetail>,
}

impl WriteHuman for StatusOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Feature `{}` (branch: {}) in {}:",
            self.name, self.branch, self.root
        )?;
        if self.repos.is_empty() {
            writeln!(w, "  no repos promoted")?;
        }
        for detail in &self.repos {
            let present = if detail.worktree_present {
                "present"
            } else {
                "missing"
            };
            writeln!(
                w,
                "  {}  {}  worktree {present}",
                detail.repo,
                state_word(detail.state),
            )?;
        }
        Ok(())
    }
}

fn state_word(state: WorktreeState) -> &'static str {
    match state {
        WorktreeState::Pending => "pending",
        WorktreeState::Ready => "ready",
        WorktreeState::Failed => "failed",
    }
}

/// Show `input.feature` in detail.
///
/// `worktree_present` is a live probe — the record may say `Ready` while the
/// worktree was deleted behind `ivar`'s back, which is exactly the kind of
/// drift `doctor` exists to catch and this command exists to surface.
pub fn status(ctx: &Ctx, input: StatusInput) -> Outcome<StatusOutcome> {
    let layout = discover_hall(ctx)?;
    let git = git::System;
    let name = FeatureName::new(input.feature)?;

    let feature = Feature::read(&layout, &name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
        .expected("an existing feature")
        .actual(format!("`{name}` has no feature.json"))
        .fix(FixAction::safe(
            "feature.create_first",
            format!("Create it first with `ivar feature create {name}`."),
        ))
    })?;

    let mut repos = Vec::new();
    for (repo, promotion) in &feature.promotions {
        let worktree = layout.repo_worktree(repo, &feature.branch);
        let present = matches!(
            git.target_state(&worktree).unwrap_or(TargetState::Absent),
            TargetState::Repository
        );
        repos.push(RepoDetail {
            repo: repo.clone(),
            state: promotion.worktree,
            worktree_present: present,
        });
    }
    repos.sort_by(|a, b| a.repo.cmp(&b.repo));

    Ok(Report::new(StatusOutcome {
        root: layout.root().to_path_buf(),
        name,
        branch: feature.branch.to_string(),
        repos,
    }))
}

/// What `ivar feature status` needs.
#[derive(Debug, Clone)]
pub struct StatusInput {
    /// The feature's name.
    pub feature: String,
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/status.rs"]
mod tests;
