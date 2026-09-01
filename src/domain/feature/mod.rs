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
//! filesystem evidence. `delivery` — the guard checks and the delivery preview.
//! `integration` — the pure nested-integration vocabulary: via/strategy/override/policy, receipts and
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
//! in `feature.rs` but is declared here as `promotion` — a
//! module name matching `Feature` would clash with the type itself.
//!
//! The child module structure uses cargo-style file placement: each child is
//! declared here with explicit `mod` statements, and sibling files do not
//! share the name of the directory that contains it.

mod approval;
mod base;
mod cleanup;
mod delivery;
mod integration;
#[path = "feature.rs"]
mod promotion;
mod run;

pub use approval::{ApprovalState, Gate, GateRecord, GateState, UnknownGate};
pub use base::effective_base;
pub use cleanup::{
    BranchDeletion, CLEANUP_RECORD_SCHEMA_VERSION, CleanupApplyOutcome, CleanupApprovals,
    CleanupBlocker, CleanupFacts, CleanupPreview, CleanupRecord, CleanupRepo, CleanupRepoFacts,
    CleanupVerdict, DeliveryApproval, DeliveryBlocker, DeliveryFacts, DeliveryRepoFacts,
    DeliveryVerdict, DocumentationApproval, DocumentationDecision, TeardownApproval,
    WorktreeRemoval, classify_cleanup, classify_delivery,
};
pub use delivery::{
    DeliveryAction, DeliveryMode, DeliveryPreview, DeliveryRepo, DeliveryTreeBlocker, DraftAction,
    Guard,
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
