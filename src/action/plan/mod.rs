//! `ivar plan` — SPDD planning artifacts for a feature.
//!
//! The SPDD process produces three committed Markdown files per feature,
//! under `<hall>/plans/<feature>/`: `requirements.md`, `analysis.md`, and
//! `plan.md`. This module manages those files on disk — create, show, and
//! list — and the four approval gates around them (`approve` /
//! `invalidate`): the gates are crossed by explicit commands, recorded per
//! feature at `features/<feature>/planning/approvals.json`, and invalidated
//! by a change to an upstream artifact.
//!
//! The files are committed (they are the team's shared record of *why* a
//! feature exists), which is why the layout puts them at the hall root under
//! `plans/`, not under `.ivar/`. `status` is the read surface over the whole
//! cycle — the gates, what invalidated each, and the execution board — so the
//! SPDD state is visible without opening JSON.

use camino::Utf8PathBuf;

use crate::domain::feature::{ApprovalState, Gate};
use crate::domain::name::FeatureName;
use crate::error::Failure;
use crate::infra::{fs, hash};
use crate::store::layout::Layout;

pub mod approve;
pub mod create;
pub mod list;
pub mod show;
pub mod status;

/// The feature's approval state, or a fresh one with all four gates pending
/// if none was ever written, normalised to lifecycle order.
pub(super) fn load_approvals(
    layout: &Layout,
    feature: &FeatureName,
) -> Result<ApprovalState, Failure> {
    let mut approvals = ApprovalState::read(layout, feature)?.unwrap_or_else(ApprovalState::fresh);
    approvals.normalize();
    Ok(approvals)
}

/// The artifact a gate fingerprints. Requirements, Analysis, and Plan each
/// own their Markdown file; the Execution Graph is derived from `plan.md`, so
/// it fingerprints that too — the same content the board's graph fingerprints.
pub(super) fn artifact_path(layout: &Layout, feature: &FeatureName, gate: Gate) -> Utf8PathBuf {
    match gate {
        Gate::Requirements => layout.plan_dir(feature).join("requirements.md"),
        Gate::Analysis => layout.plan_dir(feature).join("analysis.md"),
        Gate::Plan | Gate::ExecutionGraph => layout.plan_dir(feature).join("plan.md"),
    }
}

/// SHA-256 of the gate's artifact content. `Ok(None)` when the artifact does
/// not exist — a vanished artifact is drift, not an error.
pub(super) fn artifact_fingerprint(
    layout: &Layout,
    feature: &FeatureName,
    gate: Gate,
) -> Result<Option<String>, Failure> {
    let path = artifact_path(layout, feature, gate);
    if !fs::is_file(&path)? {
        return Ok(None);
    }
    Ok(Some(hash::file(&path)?))
}
