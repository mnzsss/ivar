//! `ivar feature status <feature>` — one feature in detail: every promoted
//! repo and how far its promotion got.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{Feature, WorktreeState, effective_base};
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::git::{self, Git, TargetState};

use super::super::{discover_hall, read_manifest};
use super::base;
use super::relations::TreeEntry;
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
    /// The effective base this promotion was cut from — `promote`'s recorded
    /// fact, or, for a promotion recorded before that field existed, what
    /// [`effective_base`] computes fresh. `None` only when the repo has left
    /// `ivar.json` and no base was ever recorded, so there is nothing to
    /// compute from.
    pub base: Option<BranchName>,
    /// Whether the recorded base disagrees with what the feature's current
    /// declaration would compute today — the case `promote`'s
    /// `feature.base_absent` warning covers, surfaced here where it does not
    /// scroll off screen.
    pub base_diverged: bool,
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
    /// The feature's whole subtree, in deterministic pre-order, when
    /// `--recursive` was passed: the feature at depth 0, then every
    /// descendant with its derived state, repos, and blockers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree: Option<Vec<TreeEntry>>,
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
                "  {}  {}  worktree {present}  base: {}",
                detail.repo,
                state_word(detail.state),
                base_word(detail),
            )?;
        }
        if let Some(tree) = &self.tree {
            writeln!(w, "Subtree:")?;
            for entry in tree {
                let indent = "  ".repeat(entry.depth + 1);
                let blockers = if entry.blockers.is_empty() {
                    String::new()
                } else {
                    format!("  blocked by: {}", entry.blockers.join(", "))
                };
                writeln!(
                    w,
                    "{indent}{}  state {}  repos {}{blockers}",
                    entry.feature,
                    entry.state,
                    entry.repos.len(),
                )?;
            }
        }
        Ok(())
    }
}

fn base_word(detail: &RepoDetail) -> String {
    match &detail.base {
        Some(base) if detail.base_diverged => {
            format!("{base} (diverged from the feature's declared base)")
        }
        Some(base) => base.to_string(),
        None => "unknown".to_owned(),
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
///
/// With `--recursive`, the outcome additionally carries the whole subtree in
/// deterministic pre-order, each entry with its derived state and blockers.
pub fn status(ctx: &Ctx, input: StatusInput) -> Outcome<StatusOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
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

        let manifest_repo = manifest.repos().iter().find(|r| r.name() == repo);
        let (base, base_diverged) = match manifest_repo {
            Some(manifest_repo) => {
                let default_branch = manifest_repo.default_branch();
                let expected_now = effective_base(feature.base.as_ref(), default_branch);
                let diverged = promotion
                    .base
                    .as_ref()
                    .is_some_and(|recorded| recorded != &expected_now);
                (
                    Some(base::resolve(&feature, promotion, default_branch)),
                    diverged,
                )
            }
            None => (promotion.base.clone(), false),
        };

        repos.push(RepoDetail {
            repo: repo.clone(),
            state: promotion.worktree,
            worktree_present: present,
            base,
            base_diverged,
        });
    }
    repos.sort_by(|a, b| a.repo.cmp(&b.repo));

    let tree = if input.recursive {
        Some(super::relations::subtree_status(
            &git, &layout, &manifest, &name,
        )?)
    } else {
        None
    };

    Ok(Report::new(StatusOutcome {
        root: layout.root().to_path_buf(),
        name,
        branch: feature.branch.to_string(),
        repos,
        tree,
    }))
}

/// What `ivar feature status` needs.
#[derive(Debug, Clone)]
pub struct StatusInput {
    /// The feature's name.
    pub feature: String,
    /// Render the feature's whole subtree too.
    pub recursive: bool,
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/status.rs"]
mod tests;
