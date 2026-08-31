//! The Run Receipt: one provider-coordinated execution of an approved plan.
//!
//! This replaces the retired scheduler. A scheduler coordinated dependencies,
//! provider sessions, and headless-child events. A receipt is an *audit
//! boundary*: who authorised this run, against which plan revision,
//! what the filesystem looked like when it started, what the coordinator
//! reported, and what actually changed. The provider owns the scheduling now,
//! so nothing here describes tasks, dependencies, or child processes.
//!
//! # What a receipt is for
//!
//! Four questions, none of which a provider transcript can answer durably:
//!
//! 1. **Is a run already in flight?** [`RunStatus::holds_lock`] — exactly one
//!    non-terminal receipt may exist per feature, so a second coordinator is
//!    refused rather than racing the first.
//! 2. **Which plan revision authorised it?** [`RunReceipt::plan_fingerprint`]
//!    is pinned at start and rechecked at finish; a mismatch is
//!    [`RunStatus::Diverged`], never a silent re-authorisation.
//! 3. **What did the run change?** [`RunBaseline`] at start, [`RunDiff`] at
//!    each finish checkpoint — paths, states, modes and hashes, never source
//!    bytes.
//! 4. **Who coordinated it?** [`RunReceipt::coordinators`], an ordered list of
//!    ivar session ids and providers. A run may start under Claude Code and
//!    resume under OpenCode; that is *logical* continuity of this receipt, and
//!    the provider's own conversation id is deliberately not recorded.
//!
//! # Purity
//!
//! Every transition takes its timestamp and its identity from the caller. The
//! domain never reads a clock and never mints a uuid, which is what makes the
//! transition tests deterministic and what keeps the receipt free of `store`,
//! `git`, `harness` and `cli`.
//!
//! Persisted at `features/<feature>/execution/run.json` (schema v1,
//! `Policy::Local`) by `store::feature::run`; archived receipts live under
//! `execution/archive/runs/<run-id>.json`.

use std::collections::BTreeMap;
use std::fmt;

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};

/// The schema version of `run.json`, stamped by `store::feature::run`.
pub const RUN_CURRENT_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// identity
// ---------------------------------------------------------------------------

/// A run's identity — a UUID, generated once at start and never rewritten.
///
/// Validated rather than a bare `String` because it is also a *filename*:
/// archived receipts live at `archive/runs/<run-id>.json`, and a value that
/// could hold `..` or `/` would turn `status --run <id>` into a path
/// traversal. The rule is the same one [`SessionId`] uses, for the same
/// reason.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct RunId(String);

impl RunId {
    /// Validates `value` as a UUID. The only constructor — there is no
    /// unchecked path in or out, so an id read off disk is as safe to join
    /// onto a path as one this process minted.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidRunId> {
        let value = value.into();
        uuid::Uuid::parse_str(&value).map_err(|_| InvalidRunId(value.clone()))?;
        Ok(Self(value))
    }

    /// The validated value, borrowed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RunId").field(&self.0).finish()
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RunId {
    /// Routes through [`RunId::new`]. A derived impl would let a hand-edited
    /// `run.json` smuggle a traversal past the type and straight into an
    /// archive path.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

/// A run id that is not a UUID.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a run id — expected a UUID")]
pub struct InvalidRunId(pub String);

impl From<InvalidRunId> for Failure {
    fn from(error: InvalidRunId) -> Self {
        Failure::blocked("execute.invalid_run_id", error.to_string()).fix(FixAction::safe(
            "execute.list_runs",
            "Run `ivar feature execute status <feature> --history` for the run ids that exist.",
        ))
    }
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

/// Where a run is in its lifecycle.
///
/// Three non-terminal states hold the feature's single-run lock, and three
/// terminal states release it. That split is the whole state machine: it is
/// what "may another coordinator start?" reduces to, and why recovery
/// (`blocked`, `diverged`) is deliberately not spelled as a kind of failure.
///
/// ```text
/// start ──────────────────→ active
/// active ──finish blocked─→ blocked   ──start --resume──→ active
/// active ──plan changed───→ diverged  ──accept-revision─→ blocked
/// any non-terminal ──start --restart─→ interrupted  (terminal)
/// active ──finish succeeded────────→ succeeded    (terminal)
/// active ──finish failed───────────→ failed       (terminal)
/// legacy non-terminal board ───────→ interrupted  (terminal)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// A coordinator is attached and work is in flight.
    Active,
    /// The coordinator stopped and asked for a human decision. Resumable;
    /// still holds the lock, because the work is not finished.
    Blocked,
    /// The approved plan changed under a run in flight. Resumable only after
    /// an explicit `accept-revision`; still holds the lock.
    Diverged,
    /// The coordinator reported success and the evidence was recorded.
    Succeeded,
    /// The coordinator reported failure and the evidence was recorded.
    Failed,
    /// The run stopped without a reported outcome — restarted by a human, or
    /// imported from a non-terminal legacy board.
    Interrupted,
}

impl RunStatus {
    /// Whether the run is over. A terminal receipt keeps every byte of its
    /// evidence and releases the feature's single-run lock.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Interrupted)
    }

    /// Whether this status holds the feature's single-run lock — the exact
    /// negation of [`Self::is_terminal`], named for the question callers
    /// actually ask so no call site has to re-derive it.
    #[must_use]
    pub const fn holds_lock(self) -> bool {
        !self.is_terminal()
    }
}

impl fmt::Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Active => "active",
            Self::Blocked => "blocked",
            Self::Diverged => "diverged",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        };
        f.pad(name)
    }
}

/// Where a receipt came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunProvenance {
    /// Created by `feature execute start` under the provider-native
    /// lifecycle.
    Native,
    /// Reconstructed from an execution board written before this lifecycle
    /// existed. Carries [`RunReceipt::legacy`] evidence and is always
    /// terminal.
    LegacyImport,
}

impl fmt::Display for RunProvenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Native => "native",
            Self::LegacyImport => "legacy-import",
        };
        f.pad(name)
    }
}

/// The outcome a coordinator asks `finish` to record.
///
/// Distinct from [`RunStatus`] on purpose: this is the coordinator's *claim*,
/// and finish may refuse it — a plan that moved under the run lands on
/// [`RunStatus::Diverged`] no matter which outcome was submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    /// Every operation landed and verification passed.
    Succeeded,
    /// The run cannot land its operations; a human must decide what next.
    Failed,
    /// Work stopped on a question. Recoverable — the receipt stays resumable.
    Blocked,
}

impl RunOutcome {
    /// The status this outcome produces when finish accepts it.
    #[must_use]
    pub const fn status(self) -> RunStatus {
        match self {
            Self::Succeeded => RunStatus::Succeeded,
            Self::Failed => RunStatus::Failed,
            Self::Blocked => RunStatus::Blocked,
        }
    }

    /// Parse the CLI spelling.
    pub fn parse(value: &str) -> Result<Self, UnknownRunOutcome> {
        match value {
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "blocked" => Ok(Self::Blocked),
            other => Err(UnknownRunOutcome(other.to_owned())),
        }
    }
}

impl fmt::Display for RunOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
        };
        f.pad(name)
    }
}

/// An `--outcome` value that matched no run outcome.
///
/// Named for the run rather than for the flag because `domain::feature`
/// already exports a promotion `UnknownOutcome`, and one flat namespace
/// cannot hold two.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown outcome `{0}` — expected one of: succeeded, failed, blocked")]
pub struct UnknownRunOutcome(pub String);

impl From<UnknownRunOutcome> for Failure {
    fn from(error: UnknownRunOutcome) -> Self {
        Failure::blocked("execute.unknown_outcome", error.to_string()).fix(FixAction::safe(
            "execute.valid_outcome",
            "Use one of: succeeded, failed, blocked.",
        ))
    }
}

// ---------------------------------------------------------------------------
// filesystem evidence
// ---------------------------------------------------------------------------

/// What is at one path, as far as the receipt records it.
///
/// Three states, not a `bool`: a symlink whose target changed is a real edit
/// that a file-content hash would miss entirely, and "absent" has to be a
/// value rather than a missing map entry so a *removal* can be evidence
/// rather than a gap.
///
/// Directories are never a state here — untracked directories are expanded to
/// their files before evidence is recorded, so every path in a receipt names
/// one blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathState {
    /// Nothing is at the path.
    Absent,
    /// A regular file.
    File,
    /// A symbolic link.
    Symlink,
}

/// One path's recorded state: what it is, its git filemode, and the hash of
/// its content.
///
/// **No source bytes, ever.** A hash proves a change without turning the
/// receipt into an archive of someone's working tree, which is the whole of
/// N-PRIVACY.
///
/// For a [`PathState::Symlink`] the hash is over the *link target's* bytes —
/// the same thing git stores in a symlink blob — so a state read from the
/// worktree and one read from a commit compare equal when they should.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathEvidence {
    /// What is at the path.
    pub state: PathState,
    /// The git filemode (`100644`, `100755`, `120000`), when the path exists.
    /// Recorded because flipping the executable bit is a change no content
    /// hash can see.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    /// SHA-256 of the content, when the path exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

impl PathEvidence {
    /// Nothing is at the path.
    #[must_use]
    pub const fn absent() -> Self {
        Self {
            state: PathState::Absent,
            mode: None,
            hash: None,
        }
    }

    /// A regular file with `mode` and content hash `hash`.
    #[must_use]
    pub fn file(mode: u32, hash: impl Into<String>) -> Self {
        Self {
            state: PathState::File,
            mode: Some(mode),
            hash: Some(hash.into()),
        }
    }

    /// A symlink whose target's bytes hash to `hash`.
    #[must_use]
    pub fn symlink(hash: impl Into<String>) -> Self {
        Self {
            state: PathState::Symlink,
            mode: Some(0o120_000),
            hash: Some(hash.into()),
        }
    }

    /// Whether anything is at the path.
    #[must_use]
    pub const fn exists(&self) -> bool {
        !matches!(self.state, PathState::Absent)
    }
}

/// How one path changed between a run's baseline and a finish checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// Nothing was there at start; something is there now.
    Added,
    /// Something was there at both boundaries, and its content, mode or kind
    /// differs.
    Modified,
    /// Something was there at start; nothing is there now.
    Removed,
    /// The path diverged from the starting commit at start, and now matches
    /// that commit again — inherited dirty work the run undid. Called out
    /// separately because it is neither "modified into something new" nor
    /// harmless: it destroyed work the run did not create.
    Reverted,
}

impl fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Removed => "removed",
            Self::Reverted => "reverted",
        };
        f.pad(name)
    }
}

/// Classify one path from the three states that describe it.
///
/// `start` is what the worktree held when the run began, `commit` is what the
/// run's starting commit holds for that path, and `final_state` is the
/// worktree now. `None` means the run did not change this path and it must
/// not appear in the diff — which is exactly how inherited dirty work that
/// nobody touched avoids being blamed on the run.
///
/// The `Reverted` test comes first and is deliberately narrow: the path must
/// have *diverged* from the starting commit at start (`start != commit`) and
/// match it now. A run that merely edits a clean file back and forth is
/// `None`, not `Reverted`.
#[must_use]
pub fn classify_change(
    start: &PathEvidence,
    commit: &PathEvidence,
    final_state: &PathEvidence,
) -> Option<ChangeKind> {
    if start == final_state {
        return None;
    }
    if start != commit && final_state == commit {
        return Some(ChangeKind::Reverted);
    }
    match (start.exists(), final_state.exists()) {
        (false, true) => Some(ChangeKind::Added),
        (true, false) => Some(ChangeKind::Removed),
        _ => Some(ChangeKind::Modified),
    }
}

/// One repo's state when a run started: the commit it was on, plus every path
/// that already diverged from that commit.
///
/// Clean tracked paths are deliberately absent. Their baseline content is
/// addressable from `head` for as long as the commit exists, so copying it
/// here would be storage for nothing — and the paths that *are* here are
/// exactly the ones no commit can describe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoBaseline {
    /// The worktree this baseline was read from.
    pub worktree: Utf8PathBuf,
    /// The commit `HEAD` named at start.
    pub head: String,
    /// Every path dirty or untracked at start, worktree-relative, with the
    /// state it was in. Ordered by path, so two reads render identically.
    #[serde(default)]
    pub dirty: BTreeMap<Utf8PathBuf, PathEvidence>,
}

/// Every promoted repo's state when the run started.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunBaseline {
    /// Repo name → that repo's baseline, ordered by name.
    #[serde(default)]
    pub repos: BTreeMap<String, RepoBaseline>,
}

impl RunBaseline {
    /// A baseline over no repos — what a legacy import gets, since the
    /// evidence it would need was never recorded.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }
}

/// One path's entry in a run diff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathChange {
    /// How the path changed.
    pub kind: ChangeKind,
    /// What is at the path now.
    pub final_state: PathEvidence,
}

/// What one repo looks like at a finish checkpoint, relative to its baseline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoDiff {
    /// The commit `HEAD` names now. Differs from the baseline's `head` after
    /// a commit, amend, rebase, reset, or branch switch — all of which the
    /// path set below already accounts for.
    pub head: String,
    /// Every changed path, ordered by path. One map rather than four sets:
    /// a path has exactly one classification, and four sets would let it hold
    /// two.
    #[serde(default)]
    pub changes: BTreeMap<Utf8PathBuf, PathChange>,
}

/// What every repo looks like at a finish checkpoint, relative to the run's
/// baseline.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunDiff {
    /// Repo name → that repo's diff, ordered by name.
    #[serde(default)]
    pub repos: BTreeMap<String, RepoDiff>,
}

impl RunDiff {
    /// Whether any repo changed at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.repos.values().all(|repo| repo.changes.is_empty())
    }
}

// ---------------------------------------------------------------------------
// coordinator report
// ---------------------------------------------------------------------------

/// One task the coordinator's subagents carried out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskResult {
    /// What the task was.
    pub title: String,
    /// How it ended.
    pub status: TaskStatus,
    /// What it produced, in one or two sentences.
    pub result: String,
}

/// How one reported task ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Finished, with its work landed.
    Completed,
    /// Attempted and did not land.
    Failed,
    /// Deliberately not attempted.
    Skipped,
    /// Stopped on a question a human must answer.
    Blocked,
}

/// One verification the coordinator ran.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationCheck {
    /// What was run — a command line, or the name of the check.
    pub command: String,
    /// How it ended.
    pub status: CheckStatus,
    /// What it said, condensed. Never raw output.
    pub summary: String,
}

/// How one verification ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// Ran and passed.
    Passed,
    /// Ran and failed.
    Failed,
    /// Not run.
    Skipped,
}

/// One native subagent, described in provider-neutral terms.
///
/// A *role* and a *status*, never a native child id: the identifier is
/// provider-specific, unstable, and worthless to anyone reading the receipt
/// later, which is exactly the coupling this feature removes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRole {
    /// What the subagent was asked to be — "reviewer", "test-writer".
    pub role: String,
    /// How its work ended.
    pub status: TaskStatus,
}

/// The coordinator's structured account of a run, supplied to `finish`.
///
/// Closed by `deny_unknown_fields`, which is load-bearing rather than tidy:
/// it is what stops a provider envelope, a transcript excerpt, or a native
/// session id from being smuggled in as an extra key and quietly becoming
/// ivar domain state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoordinatorReport {
    /// What happened, in prose. Required and non-blank.
    pub summary: String,
    /// What was done. At least one entry.
    pub tasks: Vec<TaskResult>,
    /// What was checked. At least one entry — a run that verified nothing has
    /// not finished, it has stopped.
    pub verification: Vec<VerificationCheck>,
    /// The subagents that ran, if the coordinator chose to describe them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentRole>,
    /// Where the run departed from the approved plan.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deviations: Vec<String>,
    /// What stopped the run, when the outcome is blocked or failed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
    /// Isolatable work deliberately left for a child feature.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub follow_ups: Vec<String>,
}

impl CoordinatorReport {
    /// Refuse a report that cannot serve as evidence.
    ///
    /// The three rules are the ones a coordinator gets wrong when it is
    /// hurrying: an empty summary, no tasks, and no verification. Each is a
    /// separate refusal because "your report is invalid" is not an actionable
    /// sentence.
    pub fn validate(&self) -> Result<(), Failure> {
        if self.summary.trim().is_empty() {
            return Err(Failure::blocked(
                "execute.report_summary_blank",
                "the coordinator report has no summary",
            )
            .fix(FixAction::safe(
                "execute.report_summary",
                "Set `summary` to a sentence describing what the run did.",
            )));
        }
        if self.tasks.is_empty() {
            return Err(Failure::blocked(
                "execute.report_no_tasks",
                "the coordinator report lists no tasks",
            )
            .fix(FixAction::safe(
                "execute.report_tasks",
                "Add at least one entry to `tasks` with a title, status, and result.",
            )));
        }
        if self.verification.is_empty() {
            return Err(Failure::blocked(
                "execute.report_no_verification",
                "the coordinator report lists no verification",
            )
            .fix(FixAction::safe(
                "execute.report_verification",
                "Add at least one entry to `verification` — the command run, its status, \
                 and what it said.",
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// lineage and checkpoints
// ---------------------------------------------------------------------------

/// One coordinator that attached to this run.
///
/// The pair is an *ivar* session and the provider it opened. Resume appends a
/// new entry rather than replacing the old one, so a run that began under
/// Claude Code and continued under OpenCode reads as two entries in order —
/// which is the honest claim. Nothing here identifies a provider-native
/// conversation, because ivar does not resume one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoordinatorEntry {
    /// The ivar feature session that coordinated.
    pub session: SessionId,
    /// The provider that session opened.
    pub provider: Provider,
    /// When it attached.
    pub attached_at: String,
}

/// What a checkpoint records about the lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckpointKind {
    /// The run was created.
    Started,
    /// A coordinator re-attached to a non-terminal run.
    Resumed,
    /// The coordinator stopped on a question.
    Blocked,
    /// The approved plan changed under the run.
    Diverged,
    /// A human adopted the new plan revision.
    RevisionAccepted,
    /// The run ended with a reported outcome.
    Terminated,
    /// The run was abandoned — restarted by a human, or imported from a
    /// non-terminal board.
    Interrupted,
    /// The receipt was reconstructed from a legacy execution board.
    LegacyImport,
}

impl fmt::Display for CheckpointKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Started => "started",
            Self::Resumed => "resumed",
            Self::Blocked => "blocked",
            Self::Diverged => "diverged",
            Self::RevisionAccepted => "revision-accepted",
            Self::Terminated => "terminated",
            Self::Interrupted => "interrupted",
            Self::LegacyImport => "legacy-import",
        };
        f.pad(name)
    }
}

/// One ordered lifecycle decision, with whatever evidence that decision
/// carried.
///
/// Checkpoints are why a blocked run can be finished twice: the first finish
/// appends a blocked checkpoint with its report and diff, a resume appends
/// another, and only the last one supplies the receipt's terminal outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunCheckpoint {
    /// When it happened.
    pub at: String,
    /// What kind of decision it was.
    pub kind: CheckpointKind,
    /// The status the receipt moved to.
    pub status: RunStatus,
    /// The coordinator session that made it. `None` for a legacy import,
    /// which no session performed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionId>,
    /// The provider that session opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
    /// The report submitted at this checkpoint, when one was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<CoordinatorReport>,
    /// The filesystem evidence captured at this checkpoint, when it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<RunDiff>,
    /// The plan fingerprint the receipt was pinned to before this checkpoint,
    /// when the checkpoint changed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_fingerprint_from: Option<String>,
    /// The plan fingerprint observed or adopted, when the checkpoint changed
    /// or compared it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_fingerprint_to: Option<String>,
}

// ---------------------------------------------------------------------------
// legacy evidence
// ---------------------------------------------------------------------------

/// One workstream copied out of an imported board, as evidence.
///
/// Deliberately flat strings: this is a *record of what the old file said*,
/// not a type the active domain reasons with. Nothing reads `status` to make
/// a decision — an imported receipt is already terminal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyWorkstream {
    /// The workstream's id in the old graph.
    pub id: String,
    /// Its human-readable title.
    pub title: String,
    /// Its last status on the board.
    pub status: String,
    /// The operations it was to run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<String>,
    /// The workstreams it waited on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
}

/// One journal entry copied out of an imported board, as evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyJournalEntry {
    /// Its total order on the board.
    pub seq: u64,
    /// When it was recorded, in the board's own format.
    pub timestamp: String,
    /// The workstream it was about; the board itself when empty.
    pub workstream: String,
    /// The kind of event.
    pub kind: String,
    /// The sentence a human reads.
    pub message: String,
}

/// Everything an imported board contributed to its receipt.
///
/// Immutable by convention and by use: nothing in the active lifecycle reads
/// or writes it after import. It exists so `status` can say *what was there*
/// without anyone having to open the archived board by hand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyEvidence {
    /// SHA-256 of the normalized board this receipt was imported from. The
    /// import is idempotent because of this value: a crash between writing
    /// the receipt and archiving the board leaves both files on disk, and
    /// this is what says "the same import, continue" rather than "a different
    /// board, refuse".
    pub source_hash: String,
    /// The board's overall status at import.
    pub board_status: String,
    /// The plan fingerprint the board's graph was derived from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_fingerprint: Option<String>,
    /// The board's workstreams, in graph order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workstreams: Vec<LegacyWorkstream>,
    /// The board's provider-session → workstream map.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sessions: BTreeMap<String, String>,
    /// The board's journal, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub journal: Vec<LegacyJournalEntry>,
    /// Where the raw normalized board was archived.
    pub archived_board: Utf8PathBuf,
}

// ---------------------------------------------------------------------------
// the receipt
// ---------------------------------------------------------------------------

/// The durable record of one provider-coordinated execution of an approved
/// plan.
///
/// Every field is either authorisation (`plan_path`, `plan_fingerprint`),
/// identity (`id`, `feature`, `provenance`, `coordinators`), lifecycle
/// (`status`, `checkpoints`, timestamps), or evidence (`baseline`,
/// `final_diff`, `outcome`, `legacy`). There is no scheduling here and there
/// is no provider state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunReceipt {
    /// The schema version — always [`RUN_CURRENT_VERSION`] for a value built
    /// here or read by `store::feature::run`.
    pub version: u32,
    /// This run's identity, stable across resume.
    pub id: RunId,
    /// The feature this run executes.
    pub feature: FeatureName,
    /// Where the receipt came from.
    pub provenance: RunProvenance,
    /// Where the run is in its lifecycle.
    pub status: RunStatus,
    /// The plan the run was authorised against, hall-relative as given.
    pub plan_path: Utf8PathBuf,
    /// SHA-256 of that plan's content at start — or at the last accepted
    /// revision.
    pub plan_fingerprint: String,
    /// When the run was created.
    pub started_at: String,
    /// When it last changed.
    pub updated_at: String,
    /// When it became terminal, if it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminated_at: Option<String>,
    /// Every coordinator that attached, in order. Never empty for a native
    /// run; empty for a legacy import.
    #[serde(default)]
    pub coordinators: Vec<CoordinatorEntry>,
    /// What the filesystem looked like at start.
    #[serde(default)]
    pub baseline: RunBaseline,
    /// Every lifecycle decision, in order.
    #[serde(default)]
    pub checkpoints: Vec<RunCheckpoint>,
    /// The evidence recorded at the terminal checkpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_diff: Option<RunDiff>,
    /// The outcome the coordinator reported, once accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<RunOutcome>,
    /// What an imported board contributed, when `provenance` is
    /// `legacy-import`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy: Option<LegacyEvidence>,
}

impl RunReceipt {
    /// Create an active run.
    ///
    /// The caller supplies the id, the timestamp, and the baseline, because
    /// all three come from outside the domain — a uuid, a clock, and a git
    /// worktree respectively. That is what makes every test below a pure
    /// value comparison.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        id: RunId,
        feature: FeatureName,
        plan_path: impl Into<Utf8PathBuf>,
        plan_fingerprint: impl Into<String>,
        baseline: RunBaseline,
        session: SessionId,
        provider: Provider,
        at: impl Into<String>,
    ) -> Self {
        let at = at.into();
        let fingerprint = plan_fingerprint.into();
        Self {
            version: RUN_CURRENT_VERSION,
            id,
            feature,
            provenance: RunProvenance::Native,
            status: RunStatus::Active,
            plan_path: plan_path.into(),
            plan_fingerprint: fingerprint.clone(),
            started_at: at.clone(),
            updated_at: at.clone(),
            terminated_at: None,
            coordinators: vec![CoordinatorEntry {
                session: session.clone(),
                provider,
                attached_at: at.clone(),
            }],
            baseline,
            checkpoints: vec![RunCheckpoint {
                at,
                kind: CheckpointKind::Started,
                status: RunStatus::Active,
                session: Some(session),
                provider: Some(provider),
                report: None,
                diff: None,
                plan_fingerprint_from: None,
                plan_fingerprint_to: Some(fingerprint),
            }],
            final_diff: None,
            outcome: None,
            legacy: None,
        }
    }

    /// Whether this run holds the feature's single-run lock.
    #[must_use]
    pub const fn holds_lock(&self) -> bool {
        self.status.holds_lock()
    }

    /// The coordinator that attached most recently, if any.
    #[must_use]
    pub fn current_coordinator(&self) -> Option<&CoordinatorEntry> {
        self.coordinators.last()
    }

    /// Attach a coordinator to a non-terminal run and make it active again.
    ///
    /// Accepts `Active` as well as `Blocked` so a coordinator whose session
    /// died mid-run can re-attach without first having to terminalize a run
    /// that was never finished. `Diverged` is refused on purpose: the plan
    /// moved, and adopting the new revision is `accept_revision`'s explicit
    /// decision, not a side effect of resuming.
    ///
    /// The lineage entry is always appended, even for the same session and
    /// provider — a receipt should record that a coordinator re-attached,
    /// and collapsing repeats would lose exactly that.
    pub fn resume(
        &mut self,
        session: SessionId,
        provider: Provider,
        at: impl Into<String>,
    ) -> Result<(), RunTransition> {
        self.require(&[RunStatus::Active, RunStatus::Blocked], "resume")?;
        let at = at.into();
        self.coordinators.push(CoordinatorEntry {
            session: session.clone(),
            provider,
            attached_at: at.clone(),
        });
        self.status = RunStatus::Active;
        self.push(RunCheckpoint {
            at,
            kind: CheckpointKind::Resumed,
            status: RunStatus::Active,
            session: Some(session),
            provider: Some(provider),
            report: None,
            diff: None,
            plan_fingerprint_from: None,
            plan_fingerprint_to: None,
        });
        Ok(())
    }

    /// Record a blocked finish: the coordinator stopped on a question.
    ///
    /// Keeps the baseline, the run id, and the lock, because the run is not
    /// over. The report and diff land on a checkpoint rather than on
    /// `final_diff`, which only a terminal checkpoint fills.
    pub fn block(
        &mut self,
        report: CoordinatorReport,
        diff: RunDiff,
        session: SessionId,
        provider: Provider,
        at: impl Into<String>,
    ) -> Result<(), RunTransition> {
        self.require(&[RunStatus::Active], "block")?;
        let at = at.into();
        self.status = RunStatus::Blocked;
        self.push(RunCheckpoint {
            at,
            kind: CheckpointKind::Blocked,
            status: RunStatus::Blocked,
            session: Some(session),
            provider: Some(provider),
            report: Some(report),
            diff: Some(diff),
            plan_fingerprint_from: None,
            plan_fingerprint_to: None,
        });
        Ok(())
    }

    /// Record that the approved plan changed under a run in flight.
    ///
    /// The submitted report is preserved — the coordinator's work is evidence
    /// whether or not its authorisation still holds — but no outcome is
    /// accepted and the pinned fingerprint is *not* rewritten. Both
    /// fingerprints go on the checkpoint so the divergence is legible after
    /// the fact.
    pub fn diverge(
        &mut self,
        observed_fingerprint: impl Into<String>,
        report: Option<CoordinatorReport>,
        session: SessionId,
        provider: Provider,
        at: impl Into<String>,
    ) -> Result<(), RunTransition> {
        self.require(&[RunStatus::Active], "diverge")?;
        let at = at.into();
        self.status = RunStatus::Diverged;
        self.push(RunCheckpoint {
            at,
            kind: CheckpointKind::Diverged,
            status: RunStatus::Diverged,
            session: Some(session),
            provider: Some(provider),
            report,
            diff: None,
            plan_fingerprint_from: Some(self.plan_fingerprint.clone()),
            plan_fingerprint_to: Some(observed_fingerprint.into()),
        });
        Ok(())
    }

    /// Adopt a newly approved plan revision for a diverged run.
    ///
    /// Lands on `Blocked`, never straight on `Active`: attaching a
    /// coordinator is `start --resume`'s job, and collapsing the two would
    /// mean a revision could be accepted by a session that then never picks
    /// the work up.
    ///
    /// A fingerprint identical to the pinned one is refused rather than
    /// accepted as a no-op — the receipt says the plan diverged, so "nothing
    /// changed" means the caller is looking at a different file than finish
    /// was.
    pub fn accept_revision(
        &mut self,
        new_fingerprint: impl Into<String>,
        session: SessionId,
        provider: Provider,
        at: impl Into<String>,
    ) -> Result<(), RunTransition> {
        self.require(&[RunStatus::Diverged], "accept-revision")?;
        let new_fingerprint = new_fingerprint.into();
        if new_fingerprint == self.plan_fingerprint {
            return Err(RunTransition::RevisionUnchanged {
                fingerprint: new_fingerprint,
            });
        }
        let previous = std::mem::replace(&mut self.plan_fingerprint, new_fingerprint.clone());
        self.status = RunStatus::Blocked;
        self.push(RunCheckpoint {
            at: at.into(),
            kind: CheckpointKind::RevisionAccepted,
            status: RunStatus::Blocked,
            session: Some(session),
            provider: Some(provider),
            report: None,
            diff: None,
            plan_fingerprint_from: Some(previous),
            plan_fingerprint_to: Some(new_fingerprint),
        });
        Ok(())
    }

    /// Terminalize the run with a reported outcome and its final evidence.
    ///
    /// Only `Succeeded` and `Failed` reach here — [`RunOutcome::Blocked`] is
    /// [`Self::block`], which is recoverable and therefore not a
    /// termination. Passing it is a caller bug, and is refused rather than
    /// quietly redirected.
    pub fn terminate(
        &mut self,
        outcome: RunOutcome,
        report: CoordinatorReport,
        diff: RunDiff,
        session: SessionId,
        provider: Provider,
        at: impl Into<String>,
    ) -> Result<(), RunTransition> {
        if outcome == RunOutcome::Blocked {
            return Err(RunTransition::BlockedIsNotTerminal);
        }
        self.require(&[RunStatus::Active], "finish")?;
        let at = at.into();
        self.status = outcome.status();
        self.outcome = Some(outcome);
        self.final_diff = Some(diff.clone());
        self.terminated_at = Some(at.clone());
        self.push(RunCheckpoint {
            at,
            kind: CheckpointKind::Terminated,
            status: outcome.status(),
            session: Some(session),
            provider: Some(provider),
            report: Some(report),
            diff: Some(diff),
            plan_fingerprint_from: None,
            plan_fingerprint_to: None,
        });
        Ok(())
    }

    /// Abandon a non-terminal run, preserving everything collected so far.
    ///
    /// What `start --restart` does before creating a fresh run, and what a
    /// non-terminal legacy board becomes on import. No outcome is set: the
    /// run reported none, and inventing one would be the dishonesty this
    /// state exists to avoid.
    pub fn interrupt(&mut self, at: impl Into<String>) -> Result<(), RunTransition> {
        if self.status.is_terminal() {
            return Err(RunTransition::AlreadyTerminal {
                status: self.status,
                operation: "restart",
            });
        }
        let at = at.into();
        self.status = RunStatus::Interrupted;
        self.terminated_at = Some(at.clone());
        let session = self
            .current_coordinator()
            .map(|entry| entry.session.clone());
        let provider = self.current_coordinator().map(|entry| entry.provider);
        self.push(RunCheckpoint {
            at,
            kind: CheckpointKind::Interrupted,
            status: RunStatus::Interrupted,
            session,
            provider,
            report: None,
            diff: None,
            plan_fingerprint_from: None,
            plan_fingerprint_to: None,
        });
        Ok(())
    }

    /// Build the receipt an imported execution board becomes.
    ///
    /// Always terminal: a board that completed keeps its outcome, and every
    /// other board — running, blocked, paused, never started — becomes
    /// `interrupted`. Nothing here claims the old workstreams can be
    /// continued, because the provider-native coordinator has no faithful
    /// mapping to their dependency, session, and write-contract state.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn from_legacy(
        id: RunId,
        feature: FeatureName,
        plan_path: impl Into<Utf8PathBuf>,
        status: RunStatus,
        outcome: Option<RunOutcome>,
        evidence: LegacyEvidence,
        at: impl Into<String>,
    ) -> Self {
        let at = at.into();
        let fingerprint = evidence.plan_fingerprint.clone().unwrap_or_default();
        Self {
            version: RUN_CURRENT_VERSION,
            id,
            feature,
            provenance: RunProvenance::LegacyImport,
            status,
            plan_path: plan_path.into(),
            plan_fingerprint: fingerprint,
            started_at: at.clone(),
            updated_at: at.clone(),
            terminated_at: Some(at.clone()),
            coordinators: Vec::new(),
            baseline: RunBaseline::empty(),
            checkpoints: vec![RunCheckpoint {
                at,
                kind: CheckpointKind::LegacyImport,
                status,
                session: None,
                provider: None,
                report: None,
                diff: None,
                plan_fingerprint_from: None,
                plan_fingerprint_to: None,
            }],
            final_diff: None,
            outcome,
            legacy: Some(evidence),
        }
    }

    /// Refuse `operation` unless the current status is one of `allowed`.
    ///
    /// One helper rather than a guard per method: every transition asks the
    /// same question, and the two refusals it can produce ("already over" and
    /// "wrong state") are the two sentences a caller can act on.
    fn require(&self, allowed: &[RunStatus], operation: &'static str) -> Result<(), RunTransition> {
        if allowed.contains(&self.status) {
            return Ok(());
        }
        if self.status.is_terminal() {
            return Err(RunTransition::AlreadyTerminal {
                status: self.status,
                operation,
            });
        }
        Err(RunTransition::WrongState {
            status: self.status,
            operation,
        })
    }

    /// Append a checkpoint and move `updated_at` with it. The one place
    /// `checkpoints` grows, so the timestamp cannot fall behind the history.
    fn push(&mut self, checkpoint: RunCheckpoint) {
        self.updated_at = checkpoint.at.clone();
        self.checkpoints.push(checkpoint);
    }
}

/// A transition the receipt's state machine refuses.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunTransition {
    /// The run is over; nothing may change it.
    #[error("this run is already {status} — `{operation}` needs a run that is still in flight")]
    AlreadyTerminal {
        /// The terminal status the run holds.
        status: RunStatus,
        /// What was attempted.
        operation: &'static str,
    },

    /// The run is live but in the wrong state for this operation.
    #[error("this run is {status} — `{operation}` is not valid from there")]
    WrongState {
        /// The status the run holds.
        status: RunStatus,
        /// What was attempted.
        operation: &'static str,
    },

    /// `accept-revision` was handed the fingerprint already pinned.
    #[error("the plan fingerprint `{fingerprint}` is the one this run is already pinned to")]
    RevisionUnchanged {
        /// The fingerprint that did not change.
        fingerprint: String,
    },

    /// A blocked outcome was routed to the terminal path.
    #[error("a blocked outcome is recoverable and does not terminate a run")]
    BlockedIsNotTerminal,
}

impl From<RunTransition> for Failure {
    fn from(error: RunTransition) -> Self {
        let what = error.to_string();
        match error {
            RunTransition::AlreadyTerminal { .. } => Failure::blocked("execute.run_terminal", what)
                .fix(FixAction::safe(
                    "execute.start_new_run",
                    "Start a new run with `ivar feature execute start <feature> --plan <path>`.",
                )),
            RunTransition::WrongState { status, .. } => {
                Failure::blocked("execute.run_wrong_state", what).fix(match status {
                    RunStatus::Diverged => FixAction::safe(
                        "execute.accept_revision",
                        "Adopt the new plan revision with \
                         `ivar feature execute accept-revision <feature> --plan <path>`.",
                    ),
                    RunStatus::Blocked => FixAction::safe(
                        "execute.resume_run",
                        "Re-attach with \
                         `ivar feature execute start <feature> --plan <path> --resume`.",
                    ),
                    _ => FixAction::safe(
                        "execute.inspect_run",
                        "Inspect the run with `ivar feature execute status <feature>`.",
                    ),
                })
            }
            RunTransition::RevisionUnchanged { .. } => {
                Failure::blocked("execute.revision_unchanged", what).fix(FixAction::safe(
                    "execute.reapprove_plan",
                    "Re-approve the plan gate so the pinned fingerprint has something to move \
                     to, then run accept-revision again.",
                ))
            }
            RunTransition::BlockedIsNotTerminal => {
                Failure::blocked("execute.blocked_not_terminal", what).fix(FixAction::safe(
                    "execute.finish_outcome",
                    "Use `--outcome succeeded` or `--outcome failed` to end the run.",
                ))
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/feature/run.rs"]
mod tests;
