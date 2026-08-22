//! Types for features and how repos are promoted into them.
//!
//! A **Feature** is one branch across the repos it has **Promoted**. The
//! feature's own `branch` name is shared by every promoted repo's worktree
//! path (`.ivar/repos/<repo>/<branch>/`); what differs per repo is whether it
//! is promoted at all, and how far the promotion got.
//!
//! # What lives here
//!
//! `promotion` — the promotion record: which repos, and each one's
//! [`WorktreeState`], plus the `FeatureBoard` approval record. `approval` —
//! the three SPDD approval gates (Requirements, Analysis, Plan) and their
//! fingerprints. `run` — the Run Receipt: one provider-coordinated execution
//! of an approved plan, its lifecycle, its coordinator lineage, and its
//! filesystem evidence. `delivery` — the guard checks and the
//! delivery preview. `execution` — the plan-derived graph of workstreams plus
//! its status and journal, retained only until the board actions are gone.
//! `write_contract` — the glob-matching write
//! contract each workstream must respect, split out of `execution` because
//! it touches no board, status or journal. `integration` — the pure
//! nested-integration vocabulary: via/strategy/override/policy, receipts and
//! verification evidence, and the derived integration-state classifier. All
//! pure, no I/O — reading and writing these values is `store::feature`'s
//! job.
//!
//! # What a valid promotion is
//!
//! - A repo is either promoted or not; there is no partial record.
//! - A promoted repo starts at [`WorktreeState::Pending`] (recorded, not yet
//!   materialised) and moves to `Ready` once its worktree exists and its
//!   setup script ran — or `Failed` when the setup script failed and the next
//!   sync must retry.
//!
//! The child modules are implementation detail; the facade below is the
//! public surface. This module owns the reexports, so `domain::feature::*`
//! keeps its established names. The child holding the promotion record lives
//! in `feature.rs` but is declared here as `promotion` — a module cannot
//! share the name of the directory that contains it.

mod approval;
mod base;
mod delivery;
mod execution;
mod integration;
#[path = "feature.rs"]
mod promotion;
mod run;
mod write_contract;

pub use approval::{ApprovalState, Gate, GateRecord, GateState, UnknownGate};
pub use base::effective_base;
pub use delivery::{DeliveryAction, DeliveryPreview, DeliveryRepo, DeliveryTreeBlocker, Guard};
pub use execution::{
    ExecutionBoard, ExecutionGraph, ExecutionStatus, JournalEntry, WorkstreamDef, WorkstreamStatus,
};
pub use integration::{
    ClassificationFacts, FeatureIntegrationState, IntegrationOverride, IntegrationPolicy,
    IntegrationReceipt, IntegrationStrategy, IntegrationVia, PrCheckResult,
    UnknownIntegrationStrategy, UnknownIntegrationVia, VerificationEvidence, VerificationResult,
    classify,
};
pub use promotion::{
    Feature, FeatureBoard, Promotion, PromotionOutcome, UnknownOutcome, WorktreeState,
};
pub use run::{
    AgentRole, ChangeKind, CheckStatus, CheckpointKind, CoordinatorEntry, CoordinatorReport,
    InvalidRunId, LegacyEvidence, LegacyJournalEntry, LegacyWorkstream, PathChange, PathEvidence,
    PathState, RUN_CURRENT_VERSION, RepoBaseline, RepoDiff, RunBaseline, RunCheckpoint, RunDiff,
    RunId, RunOutcome, RunProvenance, RunReceipt, RunStatus, RunTransition, TaskResult, TaskStatus,
    UnknownRunOutcome, VerificationCheck, classify_change,
};
pub use write_contract::WriteContract;
