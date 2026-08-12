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
        ExecutorEvent::Produced { paths } => {
            board.push_journal(JournalEntry {
                seq,
                event_id: format!("produced.{session_id}.{seq}"),
                timestamp: now_epoch_seconds(),
                workstream: workstream_id,
                kind: PRODUCED.to_owned(),
                message: produced_message(&paths),
            });
        }
        ExecutorEvent::Completed { audited } => {
            board.push_journal(JournalEntry {
                seq,
                event_id: format!("session.completed.{session_id}.{seq}"),
                timestamp: now_epoch_seconds(),
                workstream: workstream_id.clone(),
                kind: "session.completed".to_owned(),
                message: format!("Session {session_id} completed"),
            });
            if !audited || has_ever_produced(board, &workstream_id) {
                for ws in &mut board.graph.workstreams {
                    if ws.id == workstream_id {
                        ws.status = WorkstreamStatus::Done;
                    }
                }
            } else {
                board.next_event_seq += 1;
                let seq = board.next_event_seq;
                board.push_journal(JournalEntry {
                    seq,
                    event_id: format!("session.unproductive.{session_id}.{seq}"),
                    timestamp: now_epoch_seconds(),
                    workstream: workstream_id.clone(),
                    kind: "session.unproductive".to_owned(),
                    message: format!(
                        "Session {session_id} exited cleanly having changed nothing under \
                         workstream `{workstream_id}`'s write contract, and no earlier run of it \
                         did either — so there is no work behind a `done`. Nothing was reverted; \
                         inspect the worktrees and the session transcript."
                    ),
                });
                block_on(board, &workstream_id);
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

/// The journal `kind` a run's production is recorded under. The journal is
/// append-only and never pruned, which is what makes it usable as the record
/// of whether a workstream has *ever* produced — see [`has_ever_produced`].
const PRODUCED: &str = "produced";

/// How many produced paths one journal entry names before it stops counting,
/// matching the ceiling the violation message uses for the same reason: an
/// entry carrying a thousand paths is one nobody reads.
const PRODUCED_NAMED: usize = 20;

/// The `produced` entry's text — what the run changed under its own contract.
fn produced_message(paths: &[String]) -> String {
    let named: Vec<&str> = paths
        .iter()
        .take(PRODUCED_NAMED)
        .map(String::as_str)
        .collect();
    let rest = paths.len().saturating_sub(named.len());
    let tail = if rest == 0 {
        String::new()
    } else {
        format!(" (and {rest} more)")
    };
    format!(
        "Changed {} path(s) under this workstream's write contract — {}{tail}",
        paths.len(),
        named.join(", "),
    )
}

/// Has `workstream_id` produced anything under its own contract, in this run
/// or any earlier one?
///
/// # Why the journal answers this and not the run
///
/// A workstream that blocked on a question is relaunched from scratch by the
/// next `tick` (see [`super::super::prompt`]'s "Replies from a human"), and
/// the relaunched run starts from a baseline that already contains what the
/// first run wrote. So a workstream that did its job, asked one question and
/// was answered can legitimately finish its second run having changed nothing
/// new — judged per-run it would block again, be replied to again, and block
/// again, forever.
///
/// The question that survives a relaunch is therefore not "did this run
/// produce" but "has this workstream produced". The journal is already the
/// append-only record of everything that happened to the board, it is not
/// pruned, and it survives across ticks on disk — so the earlier run's
/// [`PRODUCED`] entry is still there to be found, and no new persisted field
/// (nor a `board.json` schema bump) is needed to ask.
fn has_ever_produced(board: &ExecutionBoard, workstream_id: &str) -> bool {
    board
        .journal
        .iter()
        .any(|entry| entry.workstream == workstream_id && entry.kind == PRODUCED)
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
