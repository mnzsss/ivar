//! `ivar feature demote <feature> <repo>` — remove a repo from a feature.
//!
//! Demoting removes the promotion record. The worktree stays on disk — like
//! `repo remove`, removing work can destroy uncommitted work, and that is a
//! decision `ivar cleanup` (slice 8) gets to make interactively, not a
//! config command on its own.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};

use super::super::discover_hall;
use crate::action::Ctx;

/// What `ivar feature demote` needs.
#[derive(Debug, Clone)]
pub struct DemoteInput {
    /// The feature's name.
    pub feature: String,
    /// The repo to demote.
    pub repo: String,
}

/// What `ivar feature demote` did.
#[derive(Debug, Clone, Serialize)]
pub struct DemoteOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the repo was demoted from.
    pub feature: FeatureName,
    /// The repo that was demoted.
    pub repo: RepoName,
}

impl WriteHuman for DemoteOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Demoted `{}` from feature `{}`. Its worktree stays on disk — `ivar cleanup` can remove it.",
            self.repo, self.feature,
        )
    }
}

/// Demote `input.repo` from `input.feature`.
///
/// Blocked when the feature does not exist or the repo was never promoted —
/// both name the way out, and neither leaves a half-edited record.
pub fn demote(ctx: &Ctx, input: DemoteInput) -> Outcome<DemoteOutcome> {
    let layout = discover_hall(ctx)?;
    let feature_name = FeatureName::new(input.feature)?;
    let repo_name = RepoName::new(input.repo)?;

    let mut feature =
        crate::domain::feature::Feature::read(&layout, &feature_name)?.ok_or_else(|| {
            Failure::blocked(
                "feature.not_found",
                format!("feature `{feature_name}` does not exist"),
            )
            .expected("an existing feature")
            .actual(format!("`{feature_name}` has no feature.json"))
            .fix(FixAction::safe(
                "feature.create_first",
                format!("Create it first with `ivar feature create {feature_name}`."),
            ))
        })?;

    // Removing a repo is a membership change — frozen once this promotion
    // carries a successful receipt, and by the whole-child `integrated`
    // close.
    super::mutation::ensure_promotion_mutable(&layout, &feature, &repo_name)?;

    if !feature.demote(&repo_name) {
        return Err(Failure::blocked(
            "feature.not_promoted",
            format!("`{repo_name}` is not promoted into `{feature_name}`"),
        )
        .expected("a repo currently promoted into this feature")
        .actual("this repo has no promotion record here")
        .fix(FixAction::safe(
            "feature.promote_first",
            format!("Run `ivar feature promote {feature_name} {repo_name}` first."),
        )));
    }

    feature.write(&layout)?;

    Ok(Report::new(DemoteOutcome {
        root: layout.root().to_path_buf(),
        feature: feature_name,
        repo: repo_name,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/demote.rs"]
mod tests;
