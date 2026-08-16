//! The execution board: the plan-derived `ExecutionGraph` of `WorkstreamDef`s,
//! the board's overall `ExecutionStatus`, per-workstream `WorkstreamStatus`,
//! and the append-only `JournalEntry` record. Each workstream's write
//! contract is [`WriteContract`](crate::domain::feature::WriteContract),
//! which lives in its own module — it touches no board, status or journal.
//!
//! Pure data, no I/O — persisted at `features/<feature>/execution/board.json`
//! (schema v2, `Policy::Local`) by `store::feature`.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::super::provider::Provider;

/// The schema version of `board.json`, stamped by `store::feature`.
const BOARD_CURRENT_VERSION: u32 = 2;

/// The execution board for a feature: the plan-derived graph of workstreams,
/// the board's overall status, and the append-only journal of what happened
/// to it.
///
/// Persisted per feature at `features/<feature>/execution/board.json`
/// (schema v2, `Policy::Local`) by `store::feature`. Created by
/// `feature execute prepare` from the plan and its execution graph; tick and
/// reply advance `status` and append to [`Self::journal`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBoard {
    /// The schema version — always 2 for a value built through [`Self::new`]
    /// or read by `store::feature`.
    pub version: u32,
    /// The board's overall execution status.
    pub status: ExecutionStatus,
    /// The workstream graph this board executes.
    pub graph: ExecutionGraph,
    /// Append-only record of everything that happened to the board.
    pub journal: Vec<JournalEntry>,
    /// Monotonic counter for journal entries — the total order of events.
    pub next_event_seq: u64,
    /// Which workstream blocked the board, when `status` is [`ExecutionStatus::Blocked`].
    pub blocked_by: Option<String>,
    /// Provider session id → workstream id, for running workstreams.
    pub sessions: BTreeMap<String, String>,
}

impl ExecutionBoard {
    /// A fresh board at [`ExecutionStatus::Pending`] with an empty journal,
    /// executing `graph`.
    #[must_use]
    pub fn new(graph: ExecutionGraph) -> Self {
        Self {
            version: BOARD_CURRENT_VERSION,
            status: ExecutionStatus::Pending,
            graph,
            journal: Vec::new(),
            next_event_seq: 0,
            blocked_by: None,
            sessions: BTreeMap::new(),
        }
    }

    /// Set the board's status directly, bypassing [`Self::settle`].
    /// `prepare` and `approve` use this to move through the pre-approval
    /// states ([`ExecutionStatus::AwaitingApproval`],
    /// [`ExecutionStatus::Approved`]) that `settle` deliberately never
    /// derives; `tick` and its event handlers use it to force `Running` or
    /// `Blocked` outside of a settle pass. Every other transition should go
    /// through [`Self::settle`], which recomputes status from the
    /// workstreams instead of trusting the caller.
    pub fn set_status(&mut self, status: ExecutionStatus) {
        self.status = status;
    }

    /// Append a journal entry. The journal is append-only, so this is the
    /// only way it grows.
    pub fn push_journal(&mut self, entry: JournalEntry) {
        self.journal.push(entry);
    }

    /// Recompute the board's status from its workstreams, and with it
    /// [`Self::blocked_by`].
    ///
    /// Board status is a summary of the workstreams, but every command used
    /// to set it by hand, from its own local point of view — and a command
    /// that finished its work while leaving the board `Running` left it
    /// unmovable, because `tick` only accepts `Approved`, `approve` only
    /// `AwaitingApproval` and `reply` only `Blocked`. So the summary is
    /// derived here, once, and every command that mutates workstream status
    /// calls this instead of guessing:
    ///
    /// - a `Blocked` workstream needs a human, so the board is `Blocked` —
    ///   even while siblings still run, because the block does not resolve
    ///   itself;
    /// - otherwise anything `Active` means children are alive: `Running`;
    /// - otherwise every workstream `Done` is `Completed`;
    /// - otherwise anything `Waiting` is the next wave, and `Approved` is the
    ///   status `tick` launches it from — the approval gate is not reopened,
    ///   the human approved this graph and nothing about it changed;
    /// - otherwise only `Paused` workstreams remain, waiting on
    ///   `ack-revision`: `Paused`.
    ///
    /// `Pending` and `AwaitingApproval` are not produced here: they are
    /// pre-approval states about the board itself, not about its workstreams,
    /// and only `prepare`/`approve` own them.
    pub fn settle(&mut self) {
        let workstreams = &self.graph.workstreams;
        let status = if workstreams
            .iter()
            .any(|ws| ws.status == WorkstreamStatus::Blocked)
        {
            ExecutionStatus::Blocked
        } else if workstreams
            .iter()
            .any(|ws| ws.status == WorkstreamStatus::Active)
        {
            ExecutionStatus::Running
        } else if workstreams
            .iter()
            .all(|ws| ws.status == WorkstreamStatus::Done)
        {
            ExecutionStatus::Completed
        } else if workstreams
            .iter()
            .any(|ws| ws.status == WorkstreamStatus::Waiting)
        {
            ExecutionStatus::Approved
        } else {
            ExecutionStatus::Paused
        };

        // Keep naming the workstream that actually blocked the board while it
        // is still blocked — recomputing would silently rename the blocker to
        // whichever one comes first in the graph.
        let still_blocked = self.blocked_by.as_ref().is_some_and(|id| {
            workstreams
                .iter()
                .any(|ws| ws.id == *id && ws.status == WorkstreamStatus::Blocked)
        });
        if !still_blocked {
            self.blocked_by = workstreams
                .iter()
                .find(|ws| ws.status == WorkstreamStatus::Blocked)
                .map(|ws| ws.id.clone());
        }

        self.status = status;
    }
}

/// The overall state of an execution board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    /// Board created; no workstream has started.
    Pending,
    /// Board prepared from a plan; waiting for human approval.
    AwaitingApproval,
    /// Board approved; ready to tick.
    Approved,
    /// At least one workstream is active.
    Running,
    /// Execution is halted; nothing advances until it resumes.
    Blocked,
    /// Execution is halted; nothing advances until it resumes.
    Paused,
    /// Every workstream is done.
    Completed,
    /// Execution failed and cannot continue without intervention.
    Failed,
}

impl fmt::Display for ExecutionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Pending => "pending",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Approved => "approved",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
        };
        f.pad(name)
    }
}

/// The plan-derived graph of workstreams an execution board executes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionGraph {
    /// The workstreams, in declared order.
    pub workstreams: Vec<WorkstreamDef>,
    /// SHA-256 of the `plan.md` the graph was derived from. The graph is
    /// void when the plan changes — the same content the Execution Graph
    /// approval gate fingerprints.
    pub plan_fingerprint: String,
}

/// One workstream of an execution graph: a named unit of work made of
/// operations, with ordering dependencies and a write contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkstreamDef {
    /// The workstream's id — unique within the graph.
    pub id: String,
    /// A human-readable title.
    pub title: String,
    /// The operations this workstream runs, in order.
    pub operations: Vec<String>,
    /// Ids of workstreams this one depends on — each must be done first.
    pub depends_on: Vec<String>,
    /// What this workstream is allowed to touch — the write contract.
    pub write_contract: Vec<String>,
    /// Whether the workstream has started or is still waiting.
    pub status: WorkstreamStatus,
    /// The provider to run this workstream on — `None` is the hall default.
    pub provider: Option<Provider>,
    /// The model to run this workstream with — `None` is the provider
    /// default. Reaches the provider as its own flag: `claude --model` or
    /// `opencode -m`. Distinct from [`Self::agent`] — a provider's model and
    /// agent selectors are different flags, and conflating them (as the old
    /// `tick.rs` did, rendering `agent` as `--model <agent>`) sends the wrong
    /// value to the wrong flag.
    pub model: Option<String>,
    /// The agent to run this workstream with — `None` is the provider
    /// default. Reaches the provider as its own flag, distinct from
    /// [`Self::model`]: `claude --agent` or `opencode --agent`.
    pub agent: Option<String>,
}

/// The execution state of one workstream on a board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkstreamStatus {
    /// Not yet started — either its dependencies are undone or it just has
    /// not begun.
    Waiting,
    /// At least one operation has run.
    Active,
    /// Every operation finished.
    Done,
    /// Blocked on a dependency or a fingerprint mismatch.
    Blocked,
    /// Halted by a plan revision: the plan's Operations for this workstream
    /// changed, so it stays here until a human acknowledges the new revision
    /// (`feature execute replan` pauses; `feature execute ack-revision`
    /// unpauses).
    Paused,
}

impl fmt::Display for WorkstreamStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Waiting => "waiting",
            Self::Active => "active",
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::Paused => "paused",
        };
        f.pad(name)
    }
}

/// One entry in an execution board's journal — an append-only record of what
/// happened to the board and its workstreams.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalEntry {
    /// Total order of the entry within the board — the monotonic `seq`.
    pub seq: u64,
    /// Identity of the event, for dedup — the `event_id`.
    pub event_id: String,
    /// When the entry was recorded. A string — UNIX epoch seconds today,
    /// so the format can evolve without a schema bump.
    pub timestamp: String,
    /// The workstream the entry is about; the board itself when empty.
    pub workstream: String,
    /// The kind of event: `prepared`, `started`, `completed`, `failed`, …
    pub kind: String,
    /// A human-readable sentence.
    pub message: String,
}

impl JournalEntry {
    /// A new entry stamped with the current time (UNIX epoch seconds, as a
    /// string).
    #[must_use]
    pub fn new(
        workstream: impl Into<String>,
        kind: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            seq: 0,
            event_id: String::new(),
            timestamp: now_epoch_seconds(),
            workstream: workstream.into(),
            kind: kind.into(),
            message: message.into(),
        }
    }
}

/// The current time as UNIX epoch seconds, for journal timestamps. A plain
/// `SystemTime` value rendered as a string — no clock dependency, and the
/// format can evolve later since [`JournalEntry::timestamp`] is a string.
fn now_epoch_seconds() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/feature/execution.rs"]
mod tests;
