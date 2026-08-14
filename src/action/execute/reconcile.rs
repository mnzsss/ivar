//! `ivar feature execute reconcile <feature> --workstream <id> --description
//! "..."` — fold a workstream's code divergence into the board's journal.
//!
//! # What it does
//!
//! When an executor's implementation drifts from what the plan's Operations
//! prescribed, the divergence is recorded here rather than silently
//! forgotten: this verb reads the board's journal for the workstream's prior
//! entries, captures the uncommitted `git diff` across the feature's promoted
//! worktrees, and appends a `reconcile` journal entry joining the caller's
//! description with that diff.
//!
//! **The plan is never rewritten.** Folding the divergence back into
//! `plan.md`'s Operations requires human acceptance of the changed sections
//! first — v1 records only, and the plan stays exactly as it was. Rewriting
//! it is a separate, future step; see the Valhalla definition of
//! OP-RECONCILE ("requires user acceptance before writing").

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{ExecutionBoard, Feature, JournalEntry};
use crate::domain::name::FeatureName;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::git::{self, Git};
use crate::infra::fs;
use crate::store::layout::Layout;

use super::super::discover_hall;
use super::{find_workstream, require_board};
use crate::action::Ctx;
use crate::store::feature;

/// What `ivar feature execute reconcile` needs.
#[derive(Debug, Clone)]
pub struct ReconcileInput {
    /// The feature whose board records the divergence.
    pub feature: String,
    /// The workstream the divergence belongs to.
    pub workstream: String,
    /// The executor's own description of what changed and why.
    pub description: String,
}

/// What `ivar feature execute reconcile` did.
#[derive(Debug, Clone, Serialize)]
pub struct ReconcileOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the board belongs to.
    pub feature: FeatureName,
    /// The workstream the divergence was recorded for.
    pub workstream: String,
    /// The messages of earlier journal entries for this workstream — the
    /// context the divergence is folded into.
    pub prior_deviations: Vec<String>,
    /// The uncommitted `git diff` captured across the feature's worktrees.
    pub diff: String,
    /// The board after the reconciliation.
    pub board: ExecutionBoard,
    /// Where the board was written.
    pub board_path: Utf8PathBuf,
}

impl WriteHuman for ReconcileOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Recorded reconciliation for `{}` workstream `{}` at {}",
            self.feature, self.workstream, self.board_path
        )
    }
}

/// Record a divergence for `input.workstream` in the board's journal.
///
/// Blocked when the feature has no board or the workstream is unknown. The
/// plan is never touched — only the journal grows, and the board is persisted
/// with the new entry.
pub fn reconcile(ctx: &Ctx, input: ReconcileInput) -> Outcome<ReconcileOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;

    let mut board = require_board(&layout, &feature)?;
    let board_path = feature::board_path(&layout, &feature);    let feature_record = crate::domain::feature::Feature::read(&layout, &feature)?
        .ok_or_else(|| {
            Failure::blocked(
                "execute.feature_vanished",
                format!("feature `{feature}` has a board but no feature.json"),
            )
        })?;
    crate::action::feature::ensure_not_fully_integrated(&layout, &feature_record)?;

    find_workstream(&board, &feature, &input.workstream)?;

    let prior_deviations: Vec<String> = board
        .journal
        .iter()
        .filter(|entry| entry.workstream == input.workstream)
        .map(|entry| entry.message.clone())
        .collect();
    let diff = feature_diff(&layout, &feature)?;

    let message = if diff.is_empty() {
        format!(
            "Reconciled divergence: {} (no uncommitted diff in the feature worktrees)",
            input.description
        )
    } else {
        format!("Reconciled divergence: {}\n{diff}", input.description)
    };
    board.push_journal(JournalEntry::new(&input.workstream, "reconcile", message));
    board.write(&layout, &feature)?;

    Ok(Report::new(ReconcileOutcome {
        root: layout.root().to_path_buf(),
        feature,
        workstream: input.workstream,
        prior_deviations,
        diff,
        board,
        board_path,
    }))
}

/// The uncommitted divergence across the feature's promoted worktrees: for
/// every promoted repo whose worktree exists, `git diff HEAD`, prefixed with
/// the repo name. Empty when the feature has no promoted worktrees, or none
/// of them diverge. The feature's branch comes from its promotion record, so
/// this reads the right worktree per repo.
fn feature_diff(layout: &Layout, feature_name: &FeatureName) -> Result<String, Failure> {
    let Some(feature) = Feature::read(layout, feature_name)? else {
        return Ok(String::new());
    };
    let git = git::System;
    let mut parts = Vec::new();
    for repo in feature.promotions.keys() {
        let worktree = layout.repo_worktree(repo, &feature.branch);
        if !fs::is_dir(&worktree)? {
            continue;
        }
        let diff = git.diff_worktree(&worktree)?;
        if !diff.is_empty() {
            parts.push(format!("[{repo}]\n{diff}"));
        }
    }
    Ok(parts.join("\n"))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/reconcile.rs"]
mod tests;
