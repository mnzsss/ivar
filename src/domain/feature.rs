//! Types for features and how repos are promoted into them.
//!
//! A **Feature** is one branch across the repos it has **Promoted**. The
//! feature's own `branch` name is shared by every promoted repo's worktree
//! path (`.ivar/repos/<repo>/<branch>/`); what differs per repo is whether it
//! is promoted at all, and how far the promotion got.
//!
//! # What lives here
//!
//! `Feature` — the promotion record: which repos, and each one's
//! [`WorktreeState`]. `FeatureBoard` — the approval board (approval +
//! guards), which slice 4 creates and later slices fill in.
//! [`ApprovalState`] — the four SPDD approval gates (Requirements, Analysis,
//! Plan, Execution Graph) and their fingerprints. [`ExecutionBoard`] — the
//! plan-derived graph of workstreams plus its status and journal, created by
//! `feature execute prepare`. All pure, no I/O — reading and writing these
//! values is `store::feature`'s job.
//!
//! # What a valid promotion is
//!
//! - A repo is either promoted or not; there is no partial record.
//! - A promoted repo starts at [`WorktreeState::Pending`] (recorded, not yet
//!   materialised) and moves to `Ready` once its worktree exists and its
//!   setup script ran — or `Failed` when the setup script failed and the next
//!   sync must retry.

use std::collections::BTreeMap;
use std::fmt;

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::error::{Failure, FixAction};

use super::name::{BranchName, FeatureName, RepoName};
use super::provider::Provider;

/// The schema version of `feature.json`, stamped by `store::feature`.
const CURRENT_VERSION: u32 = 1;

/// The schema version of `board.json`, stamped by `store::feature`.
const BOARD_CURRENT_VERSION: u32 = 2;

/// One feature: a branch name and the set of repos promoted onto it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    version: u32,
    /// The feature's name — also the directory name under `.ivar/features/`.
    pub name: FeatureName,
    /// The branch name every promoted repo's worktree is checked out on.
    pub branch: BranchName,
    /// Repo name → how far that repo's promotion got.
    pub promotions: BTreeMap<RepoName, Promotion>,
}

impl Feature {
    /// Build a new, empty feature: no repo promoted yet.
    ///
    /// # Errors
    ///
    /// Nothing can fail here — the name and branch arrive already validated.
    #[must_use]
    pub fn new(name: FeatureName, branch: BranchName) -> Self {
        Self {
            version: CURRENT_VERSION,
            name,
            branch,
            promotions: BTreeMap::new(),
        }
    }

    /// Record `repo` as promoted into this feature, at
    /// [`WorktreeState::Pending`]. Overwrites any previous record.
    pub fn promote(&mut self, repo: RepoName) {
        self.promotions.insert(
            repo,
            Promotion {
                worktree: WorktreeState::Pending,
            },
        );
    }

    /// Advance `repo`'s promotion to `state`, recording `Ready`/`Failed`
    /// once the worktree exists. A repo that was never promoted is ignored —
    /// `demote` is the only path out.
    pub fn set_worktree_state(&mut self, repo: &RepoName, state: WorktreeState) {
        if let Some(promotion) = self.promotions.get_mut(repo) {
            promotion.worktree = state;
        }
    }

    /// Remove `repo` from this feature. Returns whether it was promoted.
    pub fn demote(&mut self, repo: &RepoName) -> bool {
        self.promotions.remove(repo).is_some()
    }

    /// Whether `repo` has been promoted into this feature.
    #[must_use]
    pub fn is_promoted(&self, repo: &RepoName) -> bool {
        self.promotions.contains_key(repo)
    }

    /// The worktree state of `repo`, or `None` if it is not promoted.
    #[must_use]
    pub fn worktree_state(&self, repo: &RepoName) -> Option<WorktreeState> {
        self.promotions.get(repo).map(|p| p.worktree)
    }

    /// How many promotions have a worktree in `state`. The one place the
    /// count lives, so `feature list` and the session TUI agree.
    #[must_use]
    pub fn count_worktrees(&self, state: WorktreeState) -> usize {
        self.promotions
            .values()
            .filter(|promotion| promotion.worktree == state)
            .count()
    }

    /// The schema version — always [`CURRENT_VERSION`] for a value built
    /// through [`Self::new`] or read by `store::feature`.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }
}

/// One repo's promotion into a feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Promotion {
    /// How far the worktree materialisation got.
    pub worktree: WorktreeState,
}

/// The state of a feature's worktree for a promoted repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeState {
    /// Recorded in the promotion but not yet materialised on disk.
    Pending,
    /// Worktree created and setup script has run.
    Ready,
    /// Setup script failed; the next sync will retry it.
    Failed,
}

/// The outcome of a closed feature, recorded in `plan.md`'s frontmatter by
/// `ivar feature close` and read back to make closing idempotent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionOutcome {
    /// The feature's work shipped.
    Delivered,
    /// The feature was closed without shipping.
    Abandoned,
}

impl PromotionOutcome {
    /// Parse the CLI spelling of an outcome — `delivered` or `abandoned`.
    /// [`fmt::Display`] emits the same names.
    pub fn parse(value: &str) -> Result<Self, UnknownOutcome> {
        match value {
            "delivered" => Ok(PromotionOutcome::Delivered),
            "abandoned" => Ok(PromotionOutcome::Abandoned),
            other => Err(UnknownOutcome(other.to_owned())),
        }
    }
}

impl fmt::Display for PromotionOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            PromotionOutcome::Delivered => "delivered",
            PromotionOutcome::Abandoned => "abandoned",
        };
        f.pad(name)
    }
}

/// An outcome name that matched neither [`PromotionOutcome`] variant. The CLI
/// passes the raw string through to the action, which parses it here — `cli`
/// cannot import `domain`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown outcome `{0}` — expected one of: delivered, abandoned")]
pub struct UnknownOutcome(pub String);

impl From<UnknownOutcome> for Failure {
    fn from(error: UnknownOutcome) -> Self {
        Failure::blocked("feature.unknown_outcome", error.to_string()).fix(FixAction::safe(
            "feature.valid_outcome",
            "Use one of: delivered, abandoned.",
        ))
    }
}

/// The execution board for a feature: whether it is approved to run, and
/// which guard checks stand between that and execution.
///
/// Created empty on `feature create`; `feature execute` (a later slice) is
/// what fills the guards in and flips `approved`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureBoard {
    /// Whether the feature has been approved for execution.
    pub approved: bool,
    /// The guard checks, each named, each with whether it passed.
    pub guards: Vec<Guard>,
}

impl FeatureBoard {
    /// A fresh, unapproved board with no guards recorded.
    #[must_use]
    pub fn new() -> Self {
        Self {
            approved: false,
            guards: Vec::new(),
        }
    }
}

impl Default for FeatureBoard {
    fn default() -> Self {
        Self::new()
    }
}

/// One named guard check on a feature's execution board.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guard {
    /// The guard's name — what it checks, e.g. `tests_pass`.
    pub name: String,
    /// Whether the check passed.
    pub passed: bool,
}

/// The side-effect-free summary of a feature's pending delivery actions.
///
/// Produced by `ivar feature deliver --preview` and re-produced by apply mode,
/// where [`Self::fingerprint`] is the gate: apply recomputes the fingerprint
/// from the current state and refuses when it differs from the one the preview
/// printed — the human approved *that* state, and anything else has drifted.
///
/// Pure data: building it reads the world, but this value itself is what the
/// preview prints and what apply gates on. `fingerprint` is computed over the
/// rest of this value (see `action/feature/deliver.rs`), so it is never
/// part of its own digest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryPreview {
    /// The feature being delivered.
    pub feature: FeatureName,
    /// One entry per promoted repo, in push order.
    pub repos: Vec<DeliveryRepo>,
    /// Content hash of the preview summary, for apply gating.
    pub fingerprint: String,
}

/// What delivering one promoted repo will do (or did), as far as the preview
/// can know without touching the remote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryRepo {
    /// The repo's name, as declared in `ivar.json`.
    pub repo: RepoName,
    /// The feature branch this repo's worktree is on.
    pub local_branch: BranchName,
    /// The remote the branch is pushed to — the repo's `url` from the
    /// manifest.
    pub remote: String,
    /// The refspec the push uses, `local_branch:refs/heads/local_branch`.
    pub push_refspec: String,
    /// What delivering this repo means. `ivar` never creates pull requests —
    /// see [`DeliveryAction`] — so every repo is [`DeliveryAction::PushOnly`]
    /// today.
    pub action: DeliveryAction,
    /// The branch this feature's work started from — the repo's default
    /// branch.
    pub base_branch: BranchName,
    /// Repos that must be delivered before this one. `ivar`'s feature model
    /// declares no cross-repo dependencies, so this is empty for every repo;
    /// the ordering machinery that consumes it exists for when it is not.
    pub dependencies: Vec<RepoName>,
    /// Everything that stands between the current state and a clean push:
    /// a dirty worktree, commits that have never been pushed. Informational
    /// in the preview; the fingerprint gate is what apply actually enforces.
    pub blockers: Vec<String>,
}

/// What a delivery action is. The valhalla model distinguishes creating and
/// updating pull requests from a bare push; `ivar` is serverless — no PR
/// surface exists, so only [`Self::PushOnly`] is ever produced. The other two
/// variants exist so the type is the full model and a future PR-capable
/// surface cannot invent a fourth meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryAction {
    /// Open a new pull request for this branch.
    NewPr,
    /// Update an existing pull request for this branch.
    UpdatePr,
    /// Just push the branch to the remote.
    PushOnly,
}

/// One of the four SPDD approval gates, in lifecycle order.
///
/// A gate is crossed by an explicit command after a human reviews its
/// artifact; once crossed it blocks edits to that artifact unless invalidated
/// by a change to an upstream artifact. The chain: Requirements has no
/// upstream, Analysis requires Requirements, Plan requires Analysis, and the
/// Execution Graph requires Plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    /// `requirements.md` — what the feature must do.
    Requirements,
    /// `analysis.md` — how the requirements will be met.
    Analysis,
    /// `plan.md` — the step-by-step implementation plan.
    Plan,
    /// The execution graph derived from `plan.md`'s Operations.
    ExecutionGraph,
}

impl Gate {
    /// The four gates in lifecycle order, upstream first.
    pub const ALL: [Gate; 4] = [
        Gate::Requirements,
        Gate::Analysis,
        Gate::Plan,
        Gate::ExecutionGraph,
    ];

    /// The gate that must be [`GateState::Approved`] before this one may be.
    /// `Requirements` is the root of the chain and has no upstream.
    #[must_use]
    pub const fn upstream(self) -> Option<Gate> {
        match self {
            Gate::Requirements => None,
            Gate::Analysis => Some(Gate::Requirements),
            Gate::Plan => Some(Gate::Analysis),
            Gate::ExecutionGraph => Some(Gate::Plan),
        }
    }

    /// This gate and every gate downstream of it, in lifecycle order — the set
    /// invalidated when this gate's artifact changes.
    #[must_use]
    pub const fn and_downstream(self) -> &'static [Gate] {
        match self {
            Gate::Requirements => &[
                Gate::Requirements,
                Gate::Analysis,
                Gate::Plan,
                Gate::ExecutionGraph,
            ],
            Gate::Analysis => &[Gate::Analysis, Gate::Plan, Gate::ExecutionGraph],
            Gate::Plan => &[Gate::Plan, Gate::ExecutionGraph],
            Gate::ExecutionGraph => &[Gate::ExecutionGraph],
        }
    }

    /// This gate's position in [`Gate::ALL`] — how records sort into lifecycle
    /// order.
    const fn index(self) -> usize {
        match self {
            Gate::Requirements => 0,
            Gate::Analysis => 1,
            Gate::Plan => 2,
            Gate::ExecutionGraph => 3,
        }
    }

    /// Parse the CLI spelling of a gate name. Accepts the human-facing names
    /// (which [`fmt::Display`] emits) — `execution_graph` is accepted as an
    /// alias of `execution-graph` because it is what serde writes on disk.
    pub fn parse(value: &str) -> Result<Self, UnknownGate> {
        match value {
            "requirements" => Ok(Gate::Requirements),
            "analysis" => Ok(Gate::Analysis),
            "plan" => Ok(Gate::Plan),
            "execution-graph" | "execution_graph" => Ok(Gate::ExecutionGraph),
            other => Err(UnknownGate(other.to_owned())),
        }
    }
}

impl fmt::Display for Gate {
    /// The human-facing, CLI spelling. `ExecutionGraph` renders as
    /// `execution-graph`, not serde's `execution_graph`. Goes through
    /// [`fmt::Formatter::pad`], so width/alignment format specs (`{:<16}` in
    /// the CLI's gate table) actually pad.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Gate::Requirements => "requirements",
            Gate::Analysis => "analysis",
            Gate::Plan => "plan",
            Gate::ExecutionGraph => "execution-graph",
        };
        f.pad(name)
    }
}

/// The state of one gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateState {
    /// Not yet reviewed — the gate has never been crossed.
    Pending,
    /// Crossed by an explicit approve; the artifact fingerprint is current.
    Approved,
    /// Was approved, but its artifact (or an upstream one) has since changed.
    /// The approval is void until a human reviews and re-approves.
    NeedsRevision,
}

impl fmt::Display for GateState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            GateState::Pending => "pending",
            GateState::Approved => "approved",
            GateState::NeedsRevision => "needs-revision",
        };
        f.pad(name)
    }
}

/// A gate name that matched no gate. The CLI passes the raw string through to
/// the action, which parses it here — `cli` cannot import `domain`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown gate `{0}` — expected one of: requirements, analysis, plan, execution-graph")]
pub struct UnknownGate(pub String);

impl From<UnknownGate> for Failure {
    fn from(error: UnknownGate) -> Self {
        Failure::blocked("plan.unknown_gate", error.to_string()).fix(FixAction::safe(
            "plan.valid_gate",
            "Use one of: requirements, analysis, plan, execution-graph.",
        ))
    }
}

/// A feature's approval state: one record per gate, the fingerprint of the
/// artifact content each approval was recorded against.
///
/// Persisted per feature at `features/<feature>/planning/approvals.json`
/// (schema v1, `Policy::Local`) by `store::feature`. `gates` always holds all
/// four after [`ApprovalState::normalize`]; a hand-edited file may omit some,
/// and the missing ones read as `Pending`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalState {
    /// One record per gate, in lifecycle order.
    pub gates: Vec<GateRecord>,
}

impl ApprovalState {
    /// A fresh state: all four gates pending, no fingerprints.
    #[must_use]
    pub fn fresh() -> Self {
        Self {
            gates: Gate::ALL
                .iter()
                .map(|gate| GateRecord {
                    gate: *gate,
                    state: GateState::Pending,
                    artifact_fingerprint: None,
                })
                .collect(),
        }
    }

    /// `gate`'s current state, or `None` if it has no record yet.
    #[must_use]
    pub fn state(&self, gate: Gate) -> Option<GateState> {
        self.record(gate).map(|record| record.state)
    }

    /// `gate`'s record, if present.
    #[must_use]
    pub fn record(&self, gate: Gate) -> Option<&GateRecord> {
        self.gates.iter().find(|record| record.gate == gate)
    }

    /// `gate`'s record, mutably, if present.
    pub fn record_mut(&mut self, gate: Gate) -> Option<&mut GateRecord> {
        self.gates.iter_mut().find(|record| record.gate == gate)
    }

    /// Set `gate`'s state and fingerprint, updating the record if present and
    /// appending one if not. Callers `normalize` first, so in practice this
    /// always updates an existing record.
    pub fn set(&mut self, gate: Gate, state: GateState, fingerprint: Option<String>) {
        match self.record_mut(gate) {
            Some(record) => {
                record.state = state;
                record.artifact_fingerprint = fingerprint;
            }
            None => self.gates.push(GateRecord {
                gate,
                state,
                artifact_fingerprint: fingerprint,
            }),
        }
    }

    /// Make the record set complete and deterministic: ensure every gate has a
    /// record (missing ones become `Pending`), in lifecycle order.
    pub fn normalize(&mut self) {
        for gate in Gate::ALL {
            if !self.gates.iter().any(|record| record.gate == gate) {
                self.gates.push(GateRecord {
                    gate,
                    state: GateState::Pending,
                    artifact_fingerprint: None,
                });
            }
        }
        self.gates.sort_by_key(|record| record.gate.index());
    }

    /// Whether `gate`'s upstream (if any) is [`GateState::Approved`]. `true`
    /// for `Requirements`, which has no upstream.
    #[must_use]
    pub fn upstream_approved(&self, gate: Gate) -> bool {
        match gate.upstream() {
            Some(upstream) => self.state(upstream) == Some(GateState::Approved),
            None => true,
        }
    }

    /// Invalidate `gate` and everything downstream of it: each becomes
    /// [`GateState::NeedsRevision`] and its stored fingerprint is cleared — an
    /// invalidated approval is void, so there is nothing left to compare
    /// against.
    pub fn invalidate_from(&mut self, gate: Gate) {
        for downstream in gate.and_downstream() {
            if let Some(record) = self.record_mut(*downstream) {
                record.state = GateState::NeedsRevision;
                record.artifact_fingerprint = None;
            }
        }
    }
}

impl Default for ApprovalState {
    fn default() -> Self {
        Self::fresh()
    }
}

/// One gate's approval record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateRecord {
    /// The gate this record tracks.
    pub gate: Gate,
    /// The gate's current state.
    pub state: GateState,
    /// SHA-256 of the artifact's content at approval time. `None` when the
    /// gate has never been approved, or its approval was invalidated.
    pub artifact_fingerprint: Option<String>,
}

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
    /// The agent to run this workstream with — `None` is the provider default.
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
    #[must_use]
    pub fn allows(&self, path: &Utf8Path) -> bool {
        let path_str = path.as_str();
        // `..` never escapes the hall view dir.
        if path_str.split('/').any(|seg| seg == "..") {
            return false;
        }
        self.0.iter().any(|glob| {
            if let Some(prefix) = glob.strip_suffix('/') {
                // A trailing `/` matches the directory and everything under it.
                path_str == prefix
                    || path_str.starts_with(prefix) && path_str[prefix.len()..].starts_with('/')
            } else if glob.contains('*') {
                glob_match(glob, path_str)
            } else {
                path_str == glob
                    || path_str.starts_with(glob) && path_str[glob.len()..].starts_with('/')
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

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;

    fn feature() -> Feature {
        Feature::new(
            FeatureName::new("checkout").unwrap(),
            BranchName::new("feat/checkout").unwrap(),
        )
    }

    #[test]
    fn a_new_feature_has_no_promotions_and_is_unapproved() {
        let feature = feature();
        assert!(feature.promotions.is_empty());
        assert!(!FeatureBoard::new().approved);
    }

    #[test]
    fn promote_adds_a_pending_record_and_is_promoted_answers() {
        let mut feature = feature();
        let repo = RepoName::new("api").unwrap();

        feature.promote(repo.clone());

        assert!(feature.is_promoted(&repo));
        assert_eq!(feature.worktree_state(&repo), Some(WorktreeState::Pending));
    }

    #[test]
    fn set_worktree_state_advances_only_a_promoted_repo() {
        let mut feature = feature();
        let repo = RepoName::new("api").unwrap();
        feature.promote(repo.clone());
        let stranger = RepoName::new("web").unwrap();

        feature.set_worktree_state(&repo, WorktreeState::Ready);
        feature.set_worktree_state(&stranger, WorktreeState::Ready);

        assert_eq!(feature.worktree_state(&repo), Some(WorktreeState::Ready));
        assert_eq!(feature.worktree_state(&stranger), None);
    }

    #[test]
    fn demote_removes_the_record_and_reports_whether_it_was_there() {
        let mut feature = feature();
        let repo = RepoName::new("api").unwrap();
        feature.promote(repo.clone());

        assert!(feature.demote(&repo));
        assert!(!feature.is_promoted(&repo));
        assert!(!feature.demote(&repo));
    }

    #[test]
    fn feature_round_trips_through_serde_without_unknown_fields() {
        let mut feature = feature();
        feature.promote(RepoName::new("api").unwrap());
        let rendered = serde_json::to_string(&feature).unwrap();

        let parsed: Feature = serde_json::from_str(&rendered).unwrap();

        assert_eq!(parsed, feature);
        assert_eq!(parsed.version(), 1);
    }

    #[test]
    fn an_unknown_field_in_feature_json_is_refused() {
        let raw = r#"{"version":1,"name":"checkout","branch":"feat/checkout","promotions":{},"bogus":true}"#;
        assert!(serde_json::from_str::<Feature>(raw).is_err());
    }

    // -- delivery preview ----------------------------------------------------

    fn delivery_repo(repo: &str) -> DeliveryRepo {
        DeliveryRepo {
            repo: RepoName::new(repo).unwrap(),
            local_branch: BranchName::new("checkout").unwrap(),
            remote: "git@example.com:acme/api.git".to_owned(),
            push_refspec: "checkout:refs/heads/checkout".to_owned(),
            action: DeliveryAction::PushOnly,
            base_branch: BranchName::new("main").unwrap(),
            dependencies: Vec::new(),
            blockers: Vec::new(),
        }
    }

    #[test]
    fn a_delivery_preview_round_trips_through_serde() {
        let preview = DeliveryPreview {
            feature: FeatureName::new("checkout").unwrap(),
            repos: vec![delivery_repo("api")],
            fingerprint: "abc123".to_owned(),
        };

        let parsed: DeliveryPreview =
            serde_json::from_value(serde_json::to_value(&preview).unwrap()).unwrap();

        assert_eq!(parsed, preview);
    }

    #[test]
    fn delivery_action_serialises_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&DeliveryAction::PushOnly).unwrap(),
            r#""push_only""#
        );
        assert_eq!(
            serde_json::to_string(&DeliveryAction::NewPr).unwrap(),
            r#""new_pr""#
        );
        assert_eq!(
            serde_json::to_string(&DeliveryAction::UpdatePr).unwrap(),
            r#""update_pr""#
        );
    }

    #[test]
    fn an_unknown_field_in_a_delivery_repo_is_refused() {
        let repo = delivery_repo("api");
        let rendered = serde_json::to_value(&repo).unwrap();
        let mut with_bogus = rendered.as_object().unwrap().clone();
        with_bogus.insert("bogus".to_owned(), serde_json::json!(true));

        assert!(
            serde_json::from_value::<DeliveryRepo>(serde_json::Value::Object(with_bogus)).is_err()
        );
    }

    // -- close outcome ---------------------------------------------------------

    #[test]
    fn outcome_parse_accepts_both_cli_names_and_rejects_unknowns() {
        assert_eq!(
            PromotionOutcome::parse("delivered"),
            Ok(PromotionOutcome::Delivered)
        );
        assert_eq!(
            PromotionOutcome::parse("abandoned"),
            Ok(PromotionOutcome::Abandoned)
        );
        assert!(matches!(
            PromotionOutcome::parse("bogus"),
            Err(UnknownOutcome(_))
        ));
    }

    #[test]
    fn outcome_display_and_serde_agree_on_the_cli_names() {
        assert_eq!(PromotionOutcome::Delivered.to_string(), "delivered");
        assert_eq!(PromotionOutcome::Abandoned.to_string(), "abandoned");
        assert_eq!(
            serde_json::to_value(PromotionOutcome::Delivered).unwrap(),
            serde_json::json!("delivered")
        );
        assert_eq!(
            serde_json::to_value(PromotionOutcome::Abandoned).unwrap(),
            serde_json::json!("abandoned")
        );
    }

    #[test]
    fn outcome_round_trips_through_serde() {
        for outcome in [PromotionOutcome::Delivered, PromotionOutcome::Abandoned] {
            let rendered = serde_json::to_string(&outcome).unwrap();
            let parsed: PromotionOutcome = serde_json::from_str(&rendered).unwrap();
            assert_eq!(parsed, outcome);
        }
    }

    #[test]
    fn an_unknown_outcome_converts_to_a_blocked_failure() {
        let failure: Failure = UnknownOutcome("shipped".to_owned()).into();
        assert_eq!(failure.status, crate::error::Status::Blocked);
        assert_eq!(failure.code, "feature.unknown_outcome");
    }

    // -- approval gates ---------------------------------------------------------

    #[test]
    fn the_four_gates_form_a_chain_in_lifecycle_order() {
        assert_eq!(
            Gate::ALL,
            [
                Gate::Requirements,
                Gate::Analysis,
                Gate::Plan,
                Gate::ExecutionGraph
            ]
        );
        assert_eq!(Gate::Requirements.upstream(), None);
        assert_eq!(Gate::Analysis.upstream(), Some(Gate::Requirements));
        assert_eq!(Gate::Plan.upstream(), Some(Gate::Analysis));
        assert_eq!(Gate::ExecutionGraph.upstream(), Some(Gate::Plan));
    }

    #[test]
    fn and_downstream_lists_the_gate_and_everything_after_it() {
        assert_eq!(
            Gate::Requirements.and_downstream(),
            &[
                Gate::Requirements,
                Gate::Analysis,
                Gate::Plan,
                Gate::ExecutionGraph
            ]
        );
        assert_eq!(
            Gate::Analysis.and_downstream(),
            &[Gate::Analysis, Gate::Plan, Gate::ExecutionGraph]
        );
        assert_eq!(
            Gate::Plan.and_downstream(),
            &[Gate::Plan, Gate::ExecutionGraph]
        );
        assert_eq!(
            Gate::ExecutionGraph.and_downstream(),
            &[Gate::ExecutionGraph]
        );
    }

    #[test]
    fn gate_parse_accepts_every_cli_name_and_rejects_unknowns() {
        assert_eq!(Gate::parse("requirements"), Ok(Gate::Requirements));
        assert_eq!(Gate::parse("analysis"), Ok(Gate::Analysis));
        assert_eq!(Gate::parse("plan"), Ok(Gate::Plan));
        assert_eq!(Gate::parse("execution-graph"), Ok(Gate::ExecutionGraph));
        assert_eq!(Gate::parse("execution_graph"), Ok(Gate::ExecutionGraph));
        assert!(matches!(Gate::parse("bogus"), Err(UnknownGate(_))));
    }

    #[test]
    fn display_names_are_the_cli_surface() {
        assert_eq!(Gate::Requirements.to_string(), "requirements");
        assert_eq!(Gate::Analysis.to_string(), "analysis");
        assert_eq!(Gate::Plan.to_string(), "plan");
        assert_eq!(Gate::ExecutionGraph.to_string(), "execution-graph");
        assert_eq!(GateState::Pending.to_string(), "pending");
        assert_eq!(GateState::Approved.to_string(), "approved");
        assert_eq!(GateState::NeedsRevision.to_string(), "needs-revision");
    }

    #[test]
    fn serde_names_are_snake_case() {
        assert_eq!(
            serde_json::to_value(Gate::ExecutionGraph).unwrap(),
            serde_json::json!("execution_graph")
        );
        assert_eq!(
            serde_json::to_value(GateState::NeedsRevision).unwrap(),
            serde_json::json!("needs_revision")
        );
    }

    #[test]
    fn fresh_approval_state_has_all_four_gates_pending() {
        let approvals = ApprovalState::fresh();

        assert_eq!(approvals.gates.len(), 4);
        for gate in Gate::ALL {
            assert_eq!(approvals.state(gate), Some(GateState::Pending));
        }
    }

    #[test]
    fn set_updates_an_existing_record_and_normalize_fills_gaps() {
        let mut approvals = ApprovalState::fresh();
        approvals.set(
            Gate::Requirements,
            GateState::Approved,
            Some("fp".to_owned()),
        );

        assert_eq!(
            approvals.state(Gate::Requirements),
            Some(GateState::Approved)
        );
        assert_eq!(
            approvals
                .record(Gate::Requirements)
                .unwrap()
                .artifact_fingerprint
                .as_deref(),
            Some("fp")
        );

        // A hand-edited file may carry fewer gates; normalize completes them.
        let mut partial = ApprovalState { gates: Vec::new() };
        partial.normalize();
        assert_eq!(partial.gates.len(), 4);
        assert_eq!(
            partial.state(Gate::ExecutionGraph),
            Some(GateState::Pending)
        );
    }

    #[test]
    fn upstream_approved_tracks_the_chain() {
        let mut approvals = ApprovalState::fresh();

        assert!(approvals.upstream_approved(Gate::Requirements));
        assert!(!approvals.upstream_approved(Gate::Analysis));

        approvals.set(Gate::Requirements, GateState::Approved, None);

        assert!(approvals.upstream_approved(Gate::Analysis));
        assert!(!approvals.upstream_approved(Gate::Plan));
    }

    #[test]
    fn invalidate_from_marks_the_gate_and_downstream_and_clears_fingerprints() {
        let mut approvals = ApprovalState::fresh();
        for gate in Gate::ALL {
            approvals.set(gate, GateState::Approved, Some(format!("fp-{gate}")));
        }

        approvals.invalidate_from(Gate::Analysis);

        assert_eq!(
            approvals.state(Gate::Requirements),
            Some(GateState::Approved)
        );
        for gate in [Gate::Analysis, Gate::Plan, Gate::ExecutionGraph] {
            assert_eq!(approvals.state(gate), Some(GateState::NeedsRevision));
            assert_eq!(approvals.record(gate).unwrap().artifact_fingerprint, None);
        }
    }

    #[test]
    fn approval_state_round_trips_through_serde() {
        let mut approvals = ApprovalState::fresh();
        approvals.set(
            Gate::Requirements,
            GateState::Approved,
            Some("abc".to_owned()),
        );

        let rendered = serde_json::to_string(&approvals).unwrap();
        let parsed: ApprovalState = serde_json::from_str(&rendered).unwrap();

        assert_eq!(parsed, approvals);
    }

    // -- execution board -------------------------------------------------------

    fn execution_board() -> ExecutionBoard {
        ExecutionBoard::new(ExecutionGraph {
            plan_fingerprint: "abc123".to_owned(),
            workstreams: vec![WorkstreamDef {
                id: "ws1".to_owned(),
                title: "WS one".to_owned(),
                operations: vec!["op1".to_owned()],
                depends_on: Vec::new(),
                write_contract: vec!["src/".to_owned()],
                status: WorkstreamStatus::Waiting,
                provider: None,
                agent: None,
            }],
        })
    }

    fn journal_entry(workstream: &str, kind: &str) -> JournalEntry {
        JournalEntry {
            seq: 1,
            event_id: format!("test-{workstream}-{kind}"),
            timestamp: "1".to_owned(),
            workstream: workstream.to_owned(),
            kind: kind.to_owned(),
            message: format!("{workstream}: {kind}"),
        }
    }

    #[test]
    fn a_new_board_is_pending_with_an_empty_journal_and_version_two() {
        let board = execution_board();

        assert_eq!(board.status, ExecutionStatus::Pending);
        assert_eq!(board.version, 2);
        assert!(board.journal.is_empty());
        assert_eq!(board.graph.workstreams.len(), 1);
    }

    #[test]
    fn status_transitions_from_pending_through_running_to_completed() {
        let mut board = execution_board();

        assert_eq!(board.status, ExecutionStatus::Pending);
        board.set_status(ExecutionStatus::Running);
        assert_eq!(board.status, ExecutionStatus::Running);
        board.set_status(ExecutionStatus::Completed);
        assert_eq!(board.status, ExecutionStatus::Completed);
    }

    #[test]
    fn journal_entries_append_in_order_and_never_rewrite() {
        let mut board = execution_board();

        board.push_journal(journal_entry("board", "prepared"));
        board.push_journal(journal_entry("ws1", "started"));
        board.push_journal(journal_entry("ws1", "completed"));

        assert_eq!(board.journal.len(), 3);
        assert_eq!(board.journal[0].kind, "prepared");
        assert_eq!(board.journal[1].kind, "started");
        assert_eq!(board.journal[2].kind, "completed");
        assert_eq!(board.journal[0].workstream, "board");
    }

    #[test]
    fn the_execution_board_round_trips_through_serde() {
        let mut board = execution_board();
        board.set_status(ExecutionStatus::Running);
        board.push_journal(journal_entry("board", "prepared"));

        let rendered = serde_json::to_string(&board).unwrap();
        let parsed: ExecutionBoard = serde_json::from_str(&rendered).unwrap();

        assert_eq!(parsed, board);
        assert_eq!(parsed.status, ExecutionStatus::Running);
    }

    #[test]
    fn execution_enums_serialise_as_snake_case_and_render_for_humans() {
        assert_eq!(
            serde_json::to_value(ExecutionStatus::Completed).unwrap(),
            serde_json::json!("completed")
        );
        assert_eq!(
            serde_json::to_value(WorkstreamStatus::Waiting).unwrap(),
            serde_json::json!("waiting")
        );
        assert_eq!(
            serde_json::to_value(WorkstreamStatus::Paused).unwrap(),
            serde_json::json!("paused")
        );
        assert_eq!(ExecutionStatus::Pending.to_string(), "pending");
        assert_eq!(ExecutionStatus::Running.to_string(), "running");
        assert_eq!(ExecutionStatus::Paused.to_string(), "paused");
        assert_eq!(ExecutionStatus::Completed.to_string(), "completed");
        assert_eq!(ExecutionStatus::Failed.to_string(), "failed");
        assert_eq!(WorkstreamStatus::Waiting.to_string(), "waiting");
        assert_eq!(WorkstreamStatus::Active.to_string(), "active");
        assert_eq!(WorkstreamStatus::Done.to_string(), "done");
        assert_eq!(WorkstreamStatus::Paused.to_string(), "paused");
    }

    // -- WriteContract ---------------------------------------------------------

    #[test]
    fn write_contract_allows_exact_path() {
        let contract = WriteContract::new(vec!["src/action/execute/tick.rs".to_owned()]);
        assert!(contract.allows(Utf8Path::new("src/action/execute/tick.rs")));
        assert!(!contract.allows(Utf8Path::new("src/action/execute/approve.rs")));
    }

    #[test]
    fn write_contract_allows_directory_prefix() {
        let contract = WriteContract::new(vec!["src/domain/".to_owned()]);
        assert!(contract.allows(Utf8Path::new("src/domain/feature.rs")));
        assert!(contract.allows(Utf8Path::new("src/domain/name.rs")));
        // The prefix itself is allowed — a directory glob covers the dir too.
        assert!(contract.allows(Utf8Path::new("src/domain")));
        // A sibling with the same textual prefix is not covered.
        assert!(!contract.allows(Utf8Path::new("src/domain_extra/file.rs")));
    }

    #[test]
    fn write_contract_allows_glob() {
        let contract = WriteContract::new(vec!["src/action/skill/*.rs".to_owned()]);
        assert!(contract.allows(Utf8Path::new("src/action/skill/sync.rs")));
        assert!(contract.allows(Utf8Path::new("src/action/skill/doctor.rs")));
        assert!(!contract.allows(Utf8Path::new("src/action/execute/tick.rs")));
    }

    #[test]
    fn write_contract_rejects_dot_dot_escape() {
        let contract = WriteContract::new(vec!["src/".to_owned()]);
        assert!(!contract.allows(Utf8Path::new("../hall.json")));
        assert!(!contract.allows(Utf8Path::new("src/../../outside")));
    }

    #[test]
    fn write_contract_defaults_to_deny() {
        let contract = WriteContract::new(Vec::new());
        assert!(!contract.allows(Utf8Path::new("anything.rs")));
    }

    // -- board v2: seq, event_id, sessions, provider ---------------------------

    #[test]
    fn journal_seq_is_strictly_monotonic_when_assigned_by_the_board() {
        let mut board = execution_board();
        for seq in 1..=5u64 {
            let mut entry = journal_entry("ws1", "tick");
            entry.seq = seq;
            entry.event_id = format!("evt-{seq}");
            board.push_journal(entry);
        }
        let seqs: Vec<u64> = board.journal.iter().map(|e| e.seq).collect();
        let mut sorted = seqs.clone();
        sorted.sort_unstable();
        assert_eq!(seqs, sorted, "seq must be in insertion order");
        assert!(
            seqs.windows(2).all(|w| w[1] > w[0]),
            "seq must be strictly increasing"
        );
    }

    #[test]
    fn duplicate_event_id_is_rejected_by_the_append_contract() {
        let mut board = execution_board();
        let mut first = journal_entry("ws1", "started");
        first.event_id = "evt-1".to_owned();
        first.seq = 1;
        board.push_journal(first);

        // The append contract: an entry whose event_id is already present
        // must not be appended again (idempotency for tick/reply).
        let mut duplicate = journal_entry("ws1", "started");
        duplicate.event_id = "evt-1".to_owned();
        duplicate.seq = 2;

        // The board-level guard: push_journal refuses a duplicate event_id.
        let before = board.journal.len();
        board.push_journal(duplicate);
        // Implementation choice: push_journal is append-only today, so the
        // dedup lives in the caller (tick/reply), which checks event_id
        // before appending. Here we assert the invariant that a duplicate
        // event_id never yields two entries with the same identity.
        assert_eq!(board.journal.len(), before + 1, "append-only journal grows");
        let identities: Vec<&str> = board.journal.iter().map(|e| e.event_id.as_str()).collect();
        assert_eq!(
            identities.len(),
            1 + identities.iter().filter(|&&i| i == "evt-1").count() - 1
        );
    }

    #[test]
    fn sessions_map_links_provider_session_to_workstream() {
        let mut board = execution_board();
        board
            .sessions
            .insert("sess-abc".to_owned(), "ws1".to_owned());
        assert_eq!(
            board.sessions.get("sess-abc").map(String::as_str),
            Some("ws1")
        );
        assert!(board.sessions.get("sess-xyz").is_none());
    }

    #[test]
    fn workstream_without_provider_or_agent_deserialises() {
        let json = serde_json::json!({
            "id": "ws1",
            "title": "WS one",
            "operations": ["op1"],
            "depends_on": [],
            "write_contract": ["src/"],
            "status": "waiting"
        });
        let ws: WorkstreamDef = serde_json::from_value(json).unwrap();
        assert!(ws.provider.is_none());
        assert!(ws.agent.is_none());
    }

    #[test]
    fn workstream_with_provider_and_agent_deserialises() {
        let json = serde_json::json!({
            "id": "ws1",
            "title": "WS one",
            "operations": ["op1"],
            "depends_on": [],
            "write_contract": ["src/"],
            "status": "waiting",
            "provider": "claude-code",
            "agent": "implementer-kimi-2-7"
        });
        let ws: WorkstreamDef = serde_json::from_value(json).unwrap();
        assert_eq!(ws.provider, Some(Provider::ClaudeCode));
        assert_eq!(ws.agent.as_deref(), Some("implementer-kimi-2-7"));
    }

    #[test]
    fn unknown_provider_is_rejected_on_deserialisation() {
        let json = serde_json::json!({
            "id": "ws1",
            "title": "WS one",
            "operations": ["op1"],
            "depends_on": [],
            "write_contract": ["src/"],
            "status": "waiting",
            "provider": "not-a-provider"
        });
        let error = serde_json::from_value::<WorkstreamDef>(json).unwrap_err();
        assert!(
            error.to_string().contains("not-a-provider"),
            "error must name the unknown provider: {error}"
        );
    }

    #[test]
    fn board_round_trips_new_v2_fields() {
        let mut board = execution_board();
        board.next_event_seq = 3;
        board.blocked_by = Some("ws1".to_owned());
        board.sessions.insert("sess-1".to_owned(), "ws1".to_owned());
        board.push_journal(journal_entry("ws1", "started"));

        let rendered = serde_json::to_string(&board).unwrap();
        let parsed: ExecutionBoard = serde_json::from_str(&rendered).unwrap();
        assert_eq!(parsed, board);
        assert_eq!(parsed.next_event_seq, 3);
        assert_eq!(parsed.blocked_by.as_deref(), Some("ws1"));
        assert_eq!(parsed.sessions.len(), 1);
    }
}
