//! `ivar feature execute replan <feature> --plan <path>` — fold a revised
//! plan into an existing execution board.
//!
//! # What it does
//!
//! Reads the feature's [`ExecutionBoard`], fingerprints the revised plan the
//! caller points at (a new `plan.md`), and compares the two. When the
//! fingerprint is unchanged there is nothing to do and the board is left
//! untouched. When it changed, the board's `plan_fingerprint` advances to the
//! new one, every workstream whose **Operations** changed is **paused** until
//! a human acknowledges the new revision ([`crate::action::execute::ack`]),
//! and the journal records the replan with the new fingerprint.
//!
//! Unaffected workstreams are untouched — their status is left exactly as it
//! was, so they continue. A workstream paused by an *earlier* replan revision
//! stays paused until it is acknowledged; pausing is the gate, and only
//! `ack-revision` lifts it.
//!
//! Replanning never rewrites the board's workstream definitions. The graph
//! keeps the operations it was prepared with, because that is the "old plan"
//! every later replan diffs against; executors read the current Operations
//! from `plan.md` itself.
//!
//! # The plan's Operations section
//!
//! The revised `plan.md` carries the new Operations in a section this verb
//! parses: a heading whose text is `Operations`, then one subheading per
//! workstream named by its id, with `- ` bullets as its operations. A
//! `write_contract:` line switches the bullets that follow to the write
//! contract. Example:
//!
//! ```text
//! ## Operations
//!
//! ### ws-board
//! - add-board-types
//! - store-board
//! write_contract:
//! - src/action/execute
//! ```
//!
//! A workstream whose subheading is absent from the revised plan counts as
//! affected — its Operations are gone, so its executor must review the change.
//! The section runs to the end of the file; every heading inside it (besides
//! the `Operations` heading itself) names a workstream.
//!
//! # v1 scope
//!
//! Affected detection is whole-workstream: any difference in `operations` or
//! `write_contract` pauses the whole workstream. No per-operation inbox
//! granularity yet.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::feature::{ExecutionBoard, JournalEntry, WorkstreamDef, WorkstreamStatus};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::{fs, hash};

use super::super::discover_hall;
use super::plan_ops::{PlanWorkstream, operations_from_plan};
use super::require_board;
use crate::action::Ctx;
use crate::store::feature;

/// What `ivar feature execute replan` needs.
#[derive(Debug, Clone)]
pub struct ReplanInput {
    /// The feature whose board is replanned.
    pub feature: String,
    /// Path to the revised `plan.md`. Resolved against the current directory.
    pub plan: String,
}

/// What `ivar feature execute replan` did.
#[derive(Debug, Clone, Serialize)]
pub struct ReplanOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the board belongs to.
    pub feature: FeatureName,
    /// SHA-256 of the revised plan — the board's new `plan_fingerprint`.
    pub fingerprint: String,
    /// `false` when the plan was unchanged and nothing was written.
    pub changed: bool,
    /// The workstreams this replan paused, in board order.
    pub affected: Vec<String>,
    /// The board after the replan.
    pub board: ExecutionBoard,
    /// Where the board was written.
    pub board_path: Utf8PathBuf,
}

impl WriteHuman for ReplanOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.changed {
            let noun = if self.affected.len() == 1 {
                "workstream"
            } else {
                "workstreams"
            };
            writeln!(
                w,
                "Replanned `{}` to {} ({} affected {noun}) at {}",
                self.feature,
                self.fingerprint,
                self.affected.len(),
                self.board_path
            )
        } else {
            writeln!(
                w,
                "Plan for `{}` unchanged ({}); nothing to replan",
                self.feature, self.fingerprint
            )
        }
    }
}

/// Fold the revised plan at `input.plan` into `input.feature`'s board.
///
/// Blocked when the feature has no board yet — replanning advances an
/// existing board; it does not create one. A plan whose fingerprint matches
/// the board's is a no-op: no journal entry, no write. Otherwise the
/// fingerprint advances, affected workstreams pause, and the replan is
/// journaled before the board is persisted.
pub fn replan(ctx: &Ctx, input: ReplanInput) -> Outcome<ReplanOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;
    let plan_path = ctx.resolve(Utf8Path::new(&input.plan));

    let mut board = require_board(&layout, &feature)?;
    let plan_text = read_plan(&plan_path)?;
    let fingerprint = hash::text(&plan_text);
    let board_path = feature::board_path(&layout, &feature);

    // Replan persists a board: blocked once the whole child closes as
    // `integrated`, and the revised wave's contracts must not reach a locked
    // promotion.
    let feature_record =
        crate::domain::feature::Feature::read(&layout, &feature)?.ok_or_else(|| {
            Failure::blocked(
                "execute.feature_vanished",
                format!("feature `{feature}` has a board but no feature.json"),
            )
        })?;
    crate::action::feature::ensure_not_fully_integrated(&layout, &feature_record)?;
    crate::action::feature::ensure_contracts_avoid_locked_promotions(
        &layout,
        &feature_record,
        &board.graph.workstreams,
    )?;

    if fingerprint == board.graph.plan_fingerprint {
        return Ok(Report::new(ReplanOutcome {
            root: layout.root().to_path_buf(),
            feature,
            fingerprint,
            changed: false,
            affected: Vec::new(),
            board,
            board_path,
        }));
    }

    let revised = operations_from_plan(&plan_text)?;
    let affected: Vec<String> = board
        .graph
        .workstreams
        .iter()
        .filter(|workstream| is_affected(workstream, &revised))
        .map(|workstream| workstream.id.clone())
        .collect();

    for workstream in &mut board.graph.workstreams {
        if affected.contains(&workstream.id) {
            workstream.status = WorkstreamStatus::Paused;
        }
    }

    board.graph.plan_fingerprint = fingerprint.clone();
    board.push_journal(JournalEntry::new(
        "board",
        "replan",
        format!(
            "Plan revised to fingerprint {fingerprint}; affected workstreams: {}",
            if affected.is_empty() {
                "none".to_owned()
            } else {
                affected.join(", ")
            }
        ),
    ));
    board.write(&layout, &feature)?;

    Ok(Report::new(ReplanOutcome {
        root: layout.root().to_path_buf(),
        feature,
        fingerprint,
        changed: true,
        affected,
        board,
        board_path,
    }))
}

/// Read the revised plan at `path`. Blocked when the file does not exist — a
/// replan against a path that has nothing to read is a mistake, not an empty
/// revision.
fn read_plan(path: &Utf8Path) -> Result<String, Failure> {
    fs::read_text(path)?.ok_or_else(|| {
        Failure::blocked("execute.plan_missing", format!("`{}` does not exist", path))
            .expected("the revised plan.md at the given path")
            .actual("no such file")
            .fix(FixAction::safe(
                "execute.provide_plan",
                "Point --plan at the revised plan.md.",
            ))
    })
}

/// Whether `workstream`'s Operations changed between the board (the old plan)
/// and the revised plan: its `operations` or `write_contract` differ, or its
/// subheading is absent from the revised Operations section entirely.
fn is_affected(workstream: &WorkstreamDef, revised: &[PlanWorkstream]) -> bool {
    match revised.iter().find(|entry| entry.id == workstream.id) {
        Some(entry) => {
            entry.operations != workstream.operations
                || entry.write_contract != workstream.write_contract
        }
        None => true,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/replan.rs"]
mod tests;
