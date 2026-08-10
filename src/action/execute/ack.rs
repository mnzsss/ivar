//! `ivar feature execute ack-revision <feature> --workstream <id>` —
//! acknowledge a plan revision for one paused workstream.
//!
//! # What it does
//!
//! A workstream that [`replan`](crate::action::execute::replan) paused is
//! gated: it must not resume until a human acknowledges the revised plan it
//! was paused for. `ack_revision` is that acknowledgment — it unpauses the
//! workstream (back to `Waiting`), records a `replan-acked` journal entry, and
//! when the last paused workstream has acknowledged, resumes the whole board
//! by advancing its status to `Running`.
//!
//! Acknowledging a workstream that is not paused is refused: there is nothing
//! to acknowledge. The gate is the `Paused` status itself, so "every affected
//! workstream has acknowledged" is simply "no workstream is paused".

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{ExecutionBoard, ExecutionStatus, JournalEntry, WorkstreamStatus};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};

use super::super::discover_hall;
use super::{require_board, workstream_not_found};
use crate::action::Ctx;
use crate::store::feature;

/// What `ivar feature execute ack-revision` needs.
#[derive(Debug, Clone)]
pub struct AckInput {
    /// The feature whose board holds the paused workstream.
    pub feature: String,
    /// The paused workstream's id.
    pub workstream: String,
}

/// What `ivar feature execute ack-revision` did.
#[derive(Debug, Clone, Serialize)]
pub struct AckOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the board belongs to.
    pub feature: FeatureName,
    /// The workstream that was unpaused.
    pub workstream: String,
    /// `true` when this was the last paused workstream — the board resumed.
    pub resumed: bool,
    /// The board after the acknowledgment.
    pub board: ExecutionBoard,
    /// Where the board was written.
    pub board_path: Utf8PathBuf,
}

impl WriteHuman for AckOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let resumed = if self.resumed {
            " — execution resumed"
        } else {
            ""
        };
        writeln!(
            w,
            "Acknowledged plan revision for `{}` workstream `{}`{resumed} at {}",
            self.feature, self.workstream, self.board_path
        )
    }
}

/// Acknowledge the plan revision for `input.workstream`, unpausing it.
///
/// Blocked when the feature has no board, the workstream is unknown, or the
/// workstream is not paused — nothing to acknowledge. Unpausing is persisted
/// with its journal entry before the outcome is returned.
pub fn ack_revision(ctx: &Ctx, input: AckInput) -> Outcome<AckOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;

    let mut board = require_board(&layout, &feature)?;
    let board_path = feature::board_path(&layout, &feature);
    let workstream = board
        .graph
        .workstreams
        .iter_mut()
        .find(|workstream| workstream.id == input.workstream)
        .ok_or_else(|| workstream_not_found(&feature, &input.workstream))?;

    if workstream.status != WorkstreamStatus::Paused {
        return Err(Failure::blocked(
            "execute.workstream_not_paused",
            format!(
                "workstream `{}` is not paused — nothing to acknowledge",
                input.workstream
            ),
        )
        .expected("a workstream paused by a plan revision")
        .actual(format!("`{}` is {}", input.workstream, workstream.status))
        .fix(FixAction::safe(
            "execute.ack_paused_only",
            "Acknowledge a workstream only after `feature execute replan` paused it.",
        )));
    }

    workstream.status = WorkstreamStatus::Waiting;
    board.push_journal(JournalEntry::new(
        &input.workstream,
        "replan-acked",
        format!(
            "Acknowledged the plan revision; workstream `{}` unpaused",
            input.workstream
        ),
    ));

    let resumed = board
        .graph
        .workstreams
        .iter()
        .all(|workstream| workstream.status != WorkstreamStatus::Paused);
    if resumed {
        board.set_status(ExecutionStatus::Running);
    }
    board.write(&layout, &feature)?;

    Ok(Report::new(AckOutcome {
        root: layout.root().to_path_buf(),
        feature,
        workstream: input.workstream,
        resumed,
        board,
        board_path,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/ack.rs"]
mod tests;
