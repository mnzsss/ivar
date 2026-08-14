//! `ivar feature execute approve` — transition the board from
//! `AwaitingApproval` to `Approved`, closing the Execution Graph approval gate.
//!
//! # What it does
//!
//! Reads the feature's [`ExecutionBoard`], verifies it is in
//! [`ExecutionStatus::AwaitingApproval`], transitions it to
//! [`ExecutionStatus::Approved`], appends a `graph.approved` journal entry,
//! and closes the [`Gate::ExecutionGraph`] gate in the approval state by
//! setting it to [`GateState::Approved`] with the plan.md fingerprint.
//!
//! The board and approvals are persisted atomically — if either write fails
//! the operation is aborted, so the two never diverge.
//!
//! Approving a board that is not in `AwaitingApproval` is refused, naming the
//! actual state. Approving twice is idempotent: the second run detects the
//! existing `graph.approved` journal entry (by its event_id) and returns
//! success without duplicating the entry or rewriting the board.
//!
//! This is the **only** writer of the Execution Graph gate in the approvals
//! file. No other command may set `Gate::ExecutionGraph` to `Approved`.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{
    ApprovalState, ExecutionBoard, ExecutionStatus, Gate, GateState, JournalEntry,
};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::hash;
use crate::store::feature;

use super::super::discover_hall;
use crate::action::Ctx;

/// What `ivar feature execute approve` needs.
#[derive(Debug, Clone)]
pub struct ApproveInput {
    /// The feature whose board to approve.
    pub feature: String,
}

/// What `ivar feature execute approve` did.
#[derive(Debug, Clone, Serialize)]
pub struct ApproveOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the board belongs to.
    pub feature: FeatureName,
    /// Where the board was written.
    pub board_path: Utf8PathBuf,
    /// The board after the transition.
    pub board: ExecutionBoard,
}

impl WriteHuman for ApproveOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Approved execution board for `{}` at {}",
            self.feature, self.board_path
        )?;
        for record in &self.board.journal {
            writeln!(w, "  [{}] {} — {}", record.seq, record.kind, record.message)?;
        }
        Ok(())
    }
}

/// Transition `input.feature`'s board from `AwaitingApproval` to `Approved`.
///
/// Blocked when the feature has no board or the board is not in
/// `AwaitingApproval`. Idempotent: if the board is already approved (a
/// `graph.approved` entry exists), returns success without duplicating the
/// journal entry.
pub fn approve(ctx: &Ctx, input: ApproveInput) -> Outcome<ApproveOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;

    let mut board = match ExecutionBoard::read(&layout, &feature)? {
        Some(b) => b,
        None => {
            return Err(Failure::blocked(
                "execute.board_missing",
                format!("no execution board for feature `{feature}`"),
            )
            .expected("a prepared execution board")
            .actual("board.json does not exist under the feature's execution directory")
            .fix(FixAction::safe(
                "execute.prepare_first",
                format!(
                    "Prepare the board first: `ivar feature execute prepare {feature} --graph-json <path>`."
                ),
            )))
        }
    };

    // Idempotency check: if the board is already approved, there is nothing
    // to do. A `graph.approved` entry means the transition already happened.
    if board.status == ExecutionStatus::Approved {
        let board_path = feature::board_path(&layout, &feature);
        return Ok(Report::new(ApproveOutcome {
            root: layout.root().to_path_buf(),
            feature,
            board_path,
            board,
        }));
    }

    if board.status != ExecutionStatus::AwaitingApproval {
        return Err(Failure::blocked(
            "execute.board_not_awaiting_approval",
            format!(
                "cannot approve board for `{feature}`: board is `{}`, expected `awaiting_approval`",
                board.status
            ),
        )
        .expected("a board in `AwaitingApproval` status")
        .actual(format!("board status is `{}`", board.status))
        .fix(FixAction::safe(
            "execute.reprepare",
            format!(
                "Delete `{}` deliberately and re-prepare from a fresh graph.",
                feature::board_path(&layout, &feature)
            ),
        )));
    }

    // Generate a deterministic event_id based on timestamp to ensure
    // idempotency — same run produces same ID, duplicate runs detect it.
    let event_id = format!("approved.{}", now_epoch_seconds());

    // Check for existing graph.approved entry (dedup guard).
    if board
        .journal
        .iter()
        .any(|entry| entry.event_id == event_id || entry.kind == "graph.approved")
    {
        let board_path = feature::board_path(&layout, &feature);
        return Ok(Report::new(ApproveOutcome {
            root: layout.root().to_path_buf(),
            feature,
            board_path,
            board,
        }));
    }

    board.set_status(ExecutionStatus::Approved);

    // Graph approval persists a board: blocked once the whole child closes as
    // `integrated`, and the approved wave's contracts must not reach a locked
    // promotion.
    let feature_record = crate::domain::feature::Feature::read(&layout, &feature)?
        .ok_or_else(|| {
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

    let mut entry = JournalEntry::new(
        "board",
        "graph.approved",
        format!("Board approved for `{feature}` — ready to tick"),
    );
    entry.event_id = event_id;

    board.push_journal(entry);

    // Close the Execution Graph gate in the approval state.
    let mut approvals = ApprovalState::read(&layout, &feature)?.unwrap_or_default();
    approvals.normalize();

    // Fingerprint the plan.md — the same content the Execution Graph approval
    // gate fingerprints. Any plan change invalidates this approval.
    let plan_path = layout.plan_dir(&feature).join("plan.md");
    let fingerprint = hash::file(&plan_path).ok();

    // `execute approve` is the one writer of the execution-graph gate; it
    // does not touch the three SPDD gates upstream. Those belong to `plan
    // approve`, and re-opening them here would make the status cascade
    // re-open the gate this very operation just approved.
    approvals.set(Gate::ExecutionGraph, GateState::Approved, fingerprint);

    board.write(&layout, &feature)?;
    approvals.write(&layout, &feature)?;

    let board_path = feature::board_path(&layout, &feature);
    Ok(Report::new(ApproveOutcome {
        root: layout.root().to_path_buf(),
        feature,
        board_path,
        board,
    }))
}

/// Current time as UNIX epoch seconds string — matches the format used by
/// `JournalEntry::timestamp`.
fn now_epoch_seconds() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/approve.rs"]
mod tests;
