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

mod checkpoint;
mod coordinator;
mod evidence;
mod id;
mod legacy;
mod receipt;
mod status;

pub const RUN_CURRENT_VERSION: u32 = 1;

pub use checkpoint::{CheckpointKind, CoordinatorEntry, RunCheckpoint, WaveProgress};
pub use coordinator::{
    AgentRole, CheckStatus, CoordinatorReport, TaskResult, TaskStatus, VerificationCheck,
};
pub use evidence::{
    ChangeKind, PathChange, PathEvidence, PathState, RepoBaseline, RepoDiff, RunBaseline, RunDiff,
    classify_change,
};
pub use id::{InvalidRunId, RunId};
pub use legacy::{LegacyEvidence, LegacyJournalEntry, LegacyWorkstream};
pub use receipt::{RunReceipt, RunTransition};
pub use status::{RunOutcome, RunProvenance, RunStatus, UnknownRunOutcome};

#[cfg(test)]
use std::collections::BTreeMap;

#[cfg(test)]
use crate::domain::name::{FeatureName, SessionId};
#[cfg(test)]
use crate::domain::provider::Provider;
#[cfg(test)]
use crate::error::Failure;
#[cfg(test)]
use camino::Utf8PathBuf;

#[cfg(test)]
#[path = "../../../../tests/unit/domain/feature/run.rs"]
mod tests;
