//! `ivar feature execute approve` — transition the board from
//! `AwaitingApproval` to `Approved`, closing the Execution Graph approval gate.
//!
//! # What it does
//!
//! Reads the feature's [`ExecutionBoard`] and approval state. An initial
//! approval transitions [`ExecutionStatus::AwaitingApproval`] to
//! [`ExecutionStatus::Approved`]; a reapproval repairs an invalidated
//! [`Gate::ExecutionGraph`] record without changing an already-operable board.
//! Both append a `graph.approved` journal entry and set the gate to
//! [`GateState::Approved`] with the plan.md fingerprint.
//!
//! The board and approvals are persisted atomically — if either write fails
//! the operation is aborted, so the two never diverge.
//!
//! Approving a board in neither `AwaitingApproval` nor `Approved` is refused,
//! naming the actual state. A matching current gate and fingerprint make a
//! repeated approval idempotent; historical journal entries never do.
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
    /// Whether this command persisted an approval or repaired an invalidated gate.
    pub changed: bool,
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
        if self.changed {
            writeln!(
                w,
                "Approved execution board for `{}` at {}",
                self.feature, self.board_path
            )?;
        } else {
            writeln!(
                w,
                "Execution board for `{}` is already approved for the current plan revision at {}",
                self.feature, self.board_path
            )?;
        }
        for record in &self.board.journal {
            writeln!(w, "  [{}] {} — {}", record.seq, record.kind, record.message)?;
        }
        Ok(())
    }
}

/// Transition `input.feature`'s board from `AwaitingApproval` to `Approved`.
///
/// Blocked when the feature has no board or cannot become approved. Idempotent
/// only when the execution-graph gate is approved for the current plan
/// fingerprint; a historical journal entry never latches approval forever.
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

    let plan_path = layout.plan_dir(&feature).join("plan.md");
    let fingerprint = hash::file(&plan_path).ok();
    let mut approvals = ApprovalState::read(&layout, &feature)?.unwrap_or_default();
    approvals.normalize();
    let gate_is_current = approvals.state(Gate::ExecutionGraph) == Some(GateState::Approved)
        && approvals
            .record(Gate::ExecutionGraph)
            .and_then(|record| record.artifact_fingerprint.as_ref())
            == fingerprint.as_ref();

    if board.status == ExecutionStatus::Approved && gate_is_current {
        let board_path = feature::board_path(&layout, &feature);
        return Ok(Report::new(ApproveOutcome {
            changed: false,
            root: layout.root().to_path_buf(),
            feature,
            board_path,
            board,
        }));
    }

    if !matches!(
        board.status,
        ExecutionStatus::AwaitingApproval | ExecutionStatus::Approved
    ) {
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

    // Generate a deterministic event id. Gate state plus fingerprint own
    // idempotency; this guard still prevents duplicate journal entries if the
    // same approval is delivered twice within a second.
    let event_id = format!(
        "approved.{}.{}",
        now_epoch_seconds(),
        fingerprint.as_deref().unwrap_or("missing-plan")
    );
    let event_already_recorded = board.journal.iter().any(|entry| entry.event_id == event_id);

    if board.status == ExecutionStatus::AwaitingApproval {
        board.set_status(ExecutionStatus::Approved);
    }

    // Graph approval persists a board: blocked once the whole child closes as
    // `integrated`, and the approved wave's contracts must not reach a locked
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

    if !event_already_recorded {
        let mut entry = JournalEntry::new(
            "board",
            "graph.approved",
            format!("Board approved for `{feature}` — ready to tick"),
        );
        entry.event_id = event_id;
        board.push_journal(entry);
    }

    // `execute approve` is the one writer of the execution-graph gate; it
    // does not touch the three SPDD gates upstream. Those belong to `plan
    // approve`, and re-opening them here would make the status cascade
    // re-open the gate this very operation just approved.
    approvals.set(Gate::ExecutionGraph, GateState::Approved, fingerprint);

    board.write(&layout, &feature)?;
    approvals.write(&layout, &feature)?;

    let board_path = feature::board_path(&layout, &feature);
    Ok(Report::new(ApproveOutcome {
        changed: true,
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
