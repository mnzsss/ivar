//! `ivar feature execute` — the feature execution board.
//!
//! v1's verbs: `prepare` turns a feature's plan and execution graph into an
//! [`ExecutionBoard`] on disk; `replan` folds a revised plan into an existing
//! board, pausing workstreams whose operations changed until they acknowledge
//! the new revision; `ack-revision` unpauses one paused workstream (and
//! resumes the board once the last one has acknowledged); `reconcile` records
//! a workstream's code divergence in the journal without rewriting the plan.

pub mod ack;
pub mod approve;
pub mod guard_check;
pub mod prepare;
pub mod reconcile;
pub mod replan;
pub mod reply;
pub mod tick;

use crate::domain::feature::{ExecutionBoard, WorkstreamDef};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction};
use crate::store::feature;
use crate::store::layout::Layout;

/// Read the feature's execution board, blocking when none exists — every
/// verb that advances an existing board starts here. A missing board is a
/// precondition, not a state to create: `prepare` is the one-shot that makes
/// one, and re-preparing would destroy the journal.
pub(super) fn require_board(
    layout: &Layout,
    feature: &FeatureName,
) -> Result<ExecutionBoard, Failure> {
    let Some(board) = ExecutionBoard::read(layout, feature)? else {
        let path = feature::board_path(layout, feature);
        return Err(Failure::blocked(
            "execute.board_missing",
            format!("`{path}` holds no execution board for `{feature}`"),
        )
        .expected("a feature with a prepared execution board")
        .actual("no board.json under the feature's execution directory")
        .fix(FixAction::safe(
            "execute.prepare_first",
            format!(
                "Prepare the board first: `ivar feature execute prepare {feature} --graph-json <path>`."
            ),
        )));
    };
    Ok(board)
}

/// The "no such workstream" refusal, shared by every verb that names one —
/// `ack-revision` and `reconcile` both gate on a workstream the board was
/// prepared with.
pub(super) fn workstream_not_found(feature: &FeatureName, id: &str) -> Failure {
    Failure::blocked(
        "execute.workstream_not_found",
        format!("workstream `{id}` is not on `{feature}`'s board"),
    )
    .expected("a workstream id the board was prepared with")
    .actual(format!("`{feature}` has no workstream named `{id}`"))
    .fix(FixAction::safe(
        "execute.valid_workstream",
        "Name a workstream from the board's graph.",
    ))
}

/// Find `id` on `board`'s graph, or build the shared "no such workstream"
/// refusal.
pub(super) fn find_workstream<'a>(
    board: &'a ExecutionBoard,
    feature: &FeatureName,
    id: &str,
) -> Result<&'a WorkstreamDef, Failure> {
    board
        .graph
        .workstreams
        .iter()
        .find(|workstream| workstream.id == id)
        .ok_or_else(|| workstream_not_found(feature, id))
}
