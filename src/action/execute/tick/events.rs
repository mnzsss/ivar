//! The event-folding half of `tick`: how one worker's [`ExecutorEvent`]
//! stream becomes journal entries and status transitions on the board. The
//! calling thread is the sole owner of the board — see `mod.rs`'s "Who
//! spawns, who owns the board".

use std::collections::BTreeMap;

use crate::domain::feature::{ExecutionBoard, ExecutionStatus, JournalEntry, WorkstreamStatus};
use crate::domain::name::FeatureName;
use crate::error::{Failure, Warning};
use crate::harness::stream::ExecutorEvent;
use crate::store::layout::Layout;

/// One [`ExecutorEvent`] from one worker, tagged with which workstream and
/// session it belongs to — the vocabulary the calling thread folds into the
/// board.
pub(super) struct ExecutorTickEvent {
    pub(super) workstream_id: String,
    pub(super) session_id: String,
    pub(super) event: ExecutorEvent,
}

/// Everything a launch worker reports to the calling thread. The caller owns
/// both the execution board and the final action report, so warnings cross the
/// same channel as provider events instead of being discarded in the worker.
pub(super) enum TickEvent {
    Executor(ExecutorTickEvent),
    Warning(Warning),
}

/// Fold one worker's [`TickEvent`] into the board. The calling thread is the
/// sole owner of the board (see the module doc); this is the only function
/// that mutates it once launches begin. A state transition forces an
/// immediate flush; `ToolUsed` only appends to the in-memory journal — see
/// the module doc's "Write cadence".
pub(super) fn apply_event(
    board: &mut ExecutionBoard,
    layout: &Layout,
    feature: &FeatureName,
    command_displays: &BTreeMap<String, String>,
    tick_event: ExecutorTickEvent,
) -> Result<(), Failure> {
    let ExecutorTickEvent {
        workstream_id,
        session_id,
        event,
    } = tick_event;
    board.next_event_seq += 1;
    let seq = board.next_event_seq;

    let mut flush = true;
    match event {
        ExecutorEvent::Started => {
            let command_display = command_displays
                .get(&session_id)
                .map(String::as_str)
                .unwrap_or_default();
            board.push_journal(JournalEntry {
                seq,
                event_id: format!("started.{session_id}"),
                timestamp: now_epoch_seconds(),
                workstream: workstream_id,
                kind: "started".to_owned(),
                message: format!("Launched session {session_id} ({command_display})"),
            });
        }
        ExecutorEvent::ToolUsed { tool, path } => {
            let message = match path {
                Some(path) => format!("{tool} {path}"),
                None => tool,
            };
            board.push_journal(JournalEntry {
                seq,
                event_id: format!("tool.used.{session_id}.{seq}"),
                timestamp: now_epoch_seconds(),
                workstream: workstream_id,
                kind: "tool.used".to_owned(),
                message,
            });
            flush = false;
        }
        ExecutorEvent::QuestionAsked { prompt: question } => {
            board.push_journal(JournalEntry {
                seq,
                event_id: format!("question.asked.{session_id}.{seq}"),
                timestamp: now_epoch_seconds(),
                workstream: workstream_id.clone(),
                kind: "question.asked".to_owned(),
                message: question,
            });
            block_on(board, &workstream_id);
        }
        ExecutorEvent::NativeSession { id } => {
            board.push_journal(JournalEntry {
                seq,
                event_id: format!("native_session.{session_id}.{seq}"),
                timestamp: now_epoch_seconds(),
                workstream: workstream_id,
                kind: "native_session".to_owned(),
                message: format!("Native session id: {id}"),
            });
        }
        ExecutorEvent::Completed => {
            board.push_journal(JournalEntry {
                seq,
                event_id: format!("session.completed.{session_id}.{seq}"),
                timestamp: now_epoch_seconds(),
                workstream: workstream_id.clone(),
                kind: "session.completed".to_owned(),
                message: format!("Session {session_id} completed"),
            });
            for ws in &mut board.graph.workstreams {
                if ws.id == workstream_id {
                    ws.status = WorkstreamStatus::Done;
                }
            }
        }
        ExecutorEvent::Failed { error } => {
            board.push_journal(JournalEntry {
                seq,
                event_id: format!("session.failed.{session_id}.{seq}"),
                timestamp: now_epoch_seconds(),
                workstream: workstream_id.clone(),
                kind: "session.failed".to_owned(),
                message: error,
            });
            block_on(board, &workstream_id);
        }
    }

    if flush {
        board.write(layout, feature)?;
    }
    Ok(())
}

/// A workstream that cannot proceed on its own — it asked a question, or its
/// process failed — moves to [`WorkstreamStatus::Blocked`], the terminal-ish
/// status closest to "stopped, needs a human" the domain model has (there is
/// no `WorkstreamStatus::Failed` — see the module doc's "Terminal status"
/// section). The board follows it to [`ExecutionStatus::Blocked`], the exact
/// transition `reply` reverses.
fn block_on(board: &mut ExecutionBoard, workstream_id: &str) {
    for ws in &mut board.graph.workstreams {
        if ws.id == workstream_id {
            ws.status = WorkstreamStatus::Blocked;
        }
    }
    board.blocked_by = Some(workstream_id.to_owned());
    board.set_status(ExecutionStatus::Blocked);
}

fn now_epoch_seconds() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs().to_string())
        .unwrap_or_default()
}
