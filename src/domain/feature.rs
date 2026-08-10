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
    /// The state of the feature's [`Gate::Plan`] — the gate delivery is
    /// conditioned on. Read from the approvals artifact, never stored on the
    /// feature: there is no lifecycle field to fall out of step with it.
    ///
    /// `--preview` reports it and refuses nothing; apply refuses anything but
    /// [`GateState::Approved`]. It is part of the fingerprinted summary, so
    /// crossing the gate after a preview reads as drift, exactly like a new
    /// commit would.
    pub plan_gate: GateState,
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
    /// What delivering this repo means — create a new pull request, update an
    /// existing one, or push only (the branch already exists on the remote but
    /// has no PR).
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
    /// The created or updated PR URL, recorded after apply. Present only when
    /// the action was [`DeliveryAction::NewPr`] or
    /// [`DeliveryAction::UpdatePr`] and the PR step succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
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

#[cfg(test)]
#[path = "../../tests/unit/domain/feature.rs"]
mod tests;
