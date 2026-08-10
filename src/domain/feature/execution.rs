//! The execution board: the plan-derived `ExecutionGraph` of `WorkstreamDef`s,
//! the board's overall `ExecutionStatus`, per-workstream `WorkstreamStatus`,
//! the `WriteContract` each workstream must respect, and the append-only
//! `JournalEntry` record.
//!
//! Pure data, no I/O — persisted at `features/<feature>/execution/board.json`
//! (schema v2, `Policy::Local`) by `store::feature`.

use std::collections::BTreeMap;
use std::fmt;

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use super::super::provider::Provider;

/// The schema version of `board.json`, stamped by `store::feature`.
const BOARD_CURRENT_VERSION: u32 = 2;

/// The execution board for a feature: the plan-derived graph of workstreams,
/// the board's overall status, and the append-only journal of what happened
/// to it.
///
/// Persisted per feature at `features/<feature>/execution/board.json`
/// (schema v1, `Policy::Local`) by `store::feature`. Created by
/// `feature execute prepare` from the plan and its execution graph; later
/// slices (tick, reply) advance `status` and append to [`Self::journal`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBoard {
    /// The schema version — always 1 for a value built through [`Self::new`]
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

    /// Advance the board's status. v1's only mutation beside the journal —
    /// nothing in v1 drives these transitions yet; tick/reply (v2) will.
    pub fn set_status(&mut self, status: ExecutionStatus) {
        self.status = status;
    }

    /// Append a journal entry. The journal is append-only, so this is the
    /// only way it grows.
    pub fn push_journal(&mut self, entry: JournalEntry) {
        self.journal.push(entry);
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

/// The write contract of a workstream: the globs its operations may touch.
///
/// Pure — no filesystem. Matching is done against an in-memory list of globs,
/// with `..` never allowed to escape the hall view dir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteContract(Vec<String>);

impl WriteContract {
    /// Build a contract from the raw glob list.
    #[must_use]
    pub fn new(globs: Vec<String>) -> Self {
        Self(globs)
    }

    /// Whether `path` is allowed by the contract. The default is to deny:
    /// an empty contract allows nothing.
    ///
    /// A glob may be relative to the hall view dir (the common case, e.g.
    /// `src/`) or absolute. A relative glob matches `path` at any depth —
    /// `/hall/src/main.rs` and `src/main.rs` both match `src/` — because the
    /// workstream never knows where the hall lives.
    #[must_use]
    pub fn allows(&self, path: &Utf8Path) -> bool {
        let path_str = path.as_str();
        // `..` never escapes the hall view dir.
        if path_str.split('/').any(|seg| seg == "..") {
            return false;
        }
        self.0.iter().any(|glob| {
            let absolute = glob.starts_with('/');
            if let Some(prefix) = glob.strip_suffix('/') {
                // A trailing `/` matches the directory and everything under it.
                let prefix = prefix.to_owned();
                if absolute {
                    path_str == prefix
                        || path_str.starts_with(&prefix)
                            && path_str[prefix.len()..].starts_with('/')
                } else {
                    // Relative: match the prefix at any depth.
                    let needle_dir = format!("/{prefix}/");
                    path_str == prefix
                        || path_str.ends_with(&format!("/{prefix}"))
                        || path_str.contains(&needle_dir)
                        || path_str.starts_with(&format!("{prefix}/"))
                }
            } else if glob.contains('*') {
                if absolute {
                    glob_match(glob, path_str)
                } else {
                    // Try the glob against every suffix so a relative glob
                    // matches at any depth.
                    let mut slice = path_str;
                    loop {
                        if glob_match(glob, slice) {
                            return true;
                        }
                        match slice.find('/') {
                            Some(idx) => slice = &slice[idx + 1..],
                            None => return false,
                        }
                    }
                }
            } else if absolute {
                path_str == glob
                    || path_str.starts_with(glob) && path_str[glob.len()..].starts_with('/')
            } else {
                // A bare relative name matches a path that ends with it.
                path_str == glob
                    || path_str.ends_with(&format!("/{glob}"))
                    || path_str.ends_with(&format!("/{glob}/"))
            }
        })
    }
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

/// Whether `path` matches a simple glob: `*` matches any run of characters,
/// and a trailing `/` matches the directory and everything under it.
fn glob_match(glob: &str, path: &str) -> bool {
    let glob = glob.trim_end_matches('/');
    if glob.is_empty() {
        return false;
    }
    // Split on the first `*` and match the literal head/tail around it.
    let Some(star) = glob.find('*') else {
        return path == glob;
    };
    let head = &glob[..star];
    let tail = &glob[star + 1..];
    if !path.starts_with(head) {
        return false;
    }
    if tail.is_empty() {
        return true;
    }
    path[head.len()..].contains(tail)
}
