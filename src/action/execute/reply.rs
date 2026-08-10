//! `ivar feature execute reply --feature <name> --session <id> --message "..."`
//! — reply to a blocked workstream.
//!
//! # What it does
//!
//! When the board is [`ExecutionStatus::Blocked`](crate::domain::feature::ExecutionStatus),
//! records a `human.replied` journal entry and appends the message line to the
//! workstream's inbox JSONL file, then returns the board to
//! [`Running`](crate::domain::feature::ExecutionStatus) /
//! [`Active`](crate::domain::feature::WorkstreamStatus).
//!
//! Replying twice with the same content is idempotent: if an entry with the
//! same `event_id` already exists in the journal, nothing is duplicated.

use std::io;
use std::time::SystemTime;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::feature::{ExecutionStatus, JournalEntry, WorkstreamStatus};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;
use crate::store::layout::Layout;

use super::super::discover_hall;
use super::{require_board, workstream_not_found};

/// What `ivar feature execute reply` needs.
#[derive(Debug, Clone)]
pub struct ReplyInput {
    /// The feature whose board to advance.
    pub feature: Option<String>,
    /// Provider session id — looked up in the board's sessions map to find the
    /// workstream to reply to.
    pub session: Option<String>,
    /// The human-readable reply content.
    pub message: String,
}

/// What `ivar feature execute reply` did.
#[derive(Debug, Clone, Serialize)]
pub struct ReplyOutcome {
    /// The workstream that was unblocked.
    pub workstream: String,
    /// The journal entry that was appended (or deduplicated).
    pub entry_seq: u64,
    /// Where the board was written.
    pub board_path: Utf8PathBuf,
}

impl WriteHuman for ReplyOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Replied to blocked workstream `{}` (seq {}) at {}",
            self.workstream, self.entry_seq, self.board_path
        )
    }
}

/// Reply to a blocked workstream identified by `input.session`.
///
/// Blocked when the board is not in [`ExecutionStatus::Blocked`] state or no
/// session/workstream mapping exists. On success the board transitions back to
/// Running/Active and the message lands in the workstream's inbox.
pub fn reply(ctx: &Ctx, input: ReplyInput) -> Outcome<ReplyOutcome> {
    // Validate arguments before touching the hall — a missing argument is a
    // caller error, not a hall problem.
    let feature_name = require_feature(&input)?;
    let session = require_session(&input)?;

    let layout = discover_hall(ctx)?;

    let mut board = require_board(&layout, &feature_name)?;
    let board_path = crate::store::feature::board_path(&layout, &feature_name);

    // Look up the session → workstream link.
    let workstream_id = match board.sessions.get(session) {
        Some(id) => id.clone(),
        None => {
            return Err(Failure::blocked(
                "execute.reply.unknown_session",
                format!("session `{session}` is not known on `{feature_name}`"),
            )
            .expected("a session id the board knows about")
            .actual(format!("no session named `{session}`"))
            .fix(FixAction::safe(
                "execute.check_sessions",
                "Check which sessions are registered on the board.",
            )));
        }
    };

    // Validate the workstream exists on the board.
    if board
        .graph
        .workstreams
        .iter()
        .all(|ws| ws.id != workstream_id)
    {
        return Err(workstream_not_found(&feature_name, &workstream_id));
    }

    // Build the event_id for idempotency.
    let event_id = build_event_id(&workstream_id, &input.message);

    // Check for duplicate event_id in the journal (idempotency). This comes
    // before the blocked check: replaying the same reply after the board
    // already moved on is a no-op, not an error.
    if let Some(existing) = board
        .journal
        .iter()
        .find(|entry| entry.event_id == event_id)
    {
        // Already recorded — return the seq of the existing entry.
        return Ok(Report::new(ReplyOutcome {
            workstream: workstream_id,
            entry_seq: existing.seq,
            board_path,
        }));
    }

    // Board must be Blocked to accept a reply.
    if board.status != ExecutionStatus::Blocked {
        return Err(Failure::blocked(
            "execute.reply.not_blocked",
            format!(
                "board for `{feature_name}` is `{}`, not blocked",
                board.status
            ),
        )
        .expected("a board in `blocked` status")
        .actual(format!("board status is `{}`", board.status))
        .fix(FixAction::safe(
            "execute.wait_for_block",
            "Wait until execution blocks before replying.",
        )));
    }

    // Assign seq from the board's monotonic counter.
    let seq = board.next_event_seq;
    board.next_event_seq += 1;

    // Append the journal entry.
    let entry = JournalEntry {
        seq,
        event_id: event_id.clone(),
        timestamp: epoch_seconds(),
        workstream: workstream_id.clone(),
        kind: "human.replied".to_owned(),
        message: input.message.clone(),
    };
    board.push_journal(entry);

    // Append the reply line to the workstream's inbox JSONL.
    append_inbox_line(&layout, &feature_name, &workstream_id, &input.message)?;

    // Clear blocked_by and transition states.
    board.blocked_by = None;
    board.set_status(ExecutionStatus::Running);

    // Unblock the workstream: set its status to Active.
    for ws in &mut board.graph.workstreams {
        if ws.id == workstream_id && ws.status == WorkstreamStatus::Blocked {
            ws.status = WorkstreamStatus::Active;
        }
    }

    // Persist the updated board.
    board.write(&layout, &feature_name)?;

    Ok(Report::new(ReplyOutcome {
        workstream: workstream_id,
        entry_seq: seq,
        board_path,
    }))
}

fn require_feature(input: &ReplyInput) -> Result<FeatureName, Failure> {
    let feature = input.feature.as_deref().ok_or_else(|| {
        Failure::blocked(
            "execute.reply.missing_feature",
            "--feature is required".to_owned(),
        )
    })?;
    Ok(FeatureName::new(feature)?)
}

fn require_session(input: &ReplyInput) -> Result<&str, Failure> {
    input.session.as_deref().ok_or_else(|| {
        Failure::blocked(
            "execute.reply.missing_session",
            "--session is required".to_owned(),
        )
    })
}

/// Build a deterministic event_id from the workstream and message for
/// idempotency. Two replies with the same content produce the same identity.
fn build_event_id(workstream: &str, message: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    workstream.hash(&mut hasher);
    hasher.write_usize(message.len());
    message.hash(&mut hasher);
    format!("human.replied.0x{:x}", hasher.finish())
}

/// Current time as UNIX epoch seconds string.
fn epoch_seconds() -> String {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}

/// Append a single JSONL line to the workstream's inbox file. Creates the
/// inbox directory if needed. Append-only — never rewrites.
fn append_inbox_line(
    layout: &Layout,
    feature: &FeatureName,
    workstream: &str,
    message: &str,
) -> Result<(), Failure> {
    let inbox_path = layout.execution_inbox(feature, workstream);

    // Ensure the inbox directory exists.
    if let Some(parent) = inbox_path.parent() {
        fs::ensure_dir(parent).map_err(Failure::from)?;
    }

    // Append one line: a JSON object with the message and a timestamp.
    let line = serde_json::json!({
        "kind": "inbox",
        "timestamp": epoch_seconds(),
        "message": message,
    });
    let line_str = format!("{line}\n");

    // OpenAppend mode: creates if absent, appends if present.
    use std::fs::OpenOptions;
    use std::io::Write;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(inbox_path.as_std_path())
        .map_err(|source| {
            Failure::failed(
                "execute.inbox_write_failed",
                format!("could not append to `{inbox_path}`"),
            )
            .actual(source.to_string())
        })?;

    file.write_all(line_str.as_bytes()).map_err(|source| {
        Failure::failed(
            "execute.inbox_write_failed",
            format!("could not append to `{inbox_path}`"),
        )
        .actual(source.to_string())
    })?;

    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/reply.rs"]
mod tests;
