//! `ivar plan` — SPDD planning artifacts for a feature.
//!
//! The SPDD process produces three committed Markdown files per feature,
//! under `<hall>/plans/<feature>/`: `requirements.md`, `analysis.md`, and
//! `plan.md`. This module manages those files on disk — create, show, and
//! list — and the three approval gates around them (`approve` /
//! `invalidate`): the gates are crossed by explicit commands, recorded per
//! feature at `features/<feature>/planning/approvals.json`, and invalidated
//! by a change to an upstream artifact.
//!
//! The files are committed (they are the team's shared record of *why* a
//! feature exists), which is why the layout puts them at the hall root under
//! `plans/`, not under `.ivar/`. `status` is the read surface over the whole
//! cycle — the gates, what invalidated each, and current run evidence — so the
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

/// The feature's approval state, or a fresh one with all three gates pending
/// if none was ever written, normalised to lifecycle order.
pub(super) fn load_approvals(
    layout: &Layout,
    feature: &FeatureName,
) -> Result<ApprovalState, Failure> {
    let mut approvals = ApprovalState::read(layout, feature)?.unwrap_or_else(ApprovalState::fresh);
    approvals.normalize();
    Ok(approvals)
}

/// The artifact each gate fingerprints. Requirements, Analysis, and Plan each
/// own one Markdown file.
pub(super) fn artifact_path(layout: &Layout, feature: &FeatureName, gate: Gate) -> Utf8PathBuf {
    match gate {
        Gate::Requirements => layout.plan_dir(feature).join("requirements.md"),
        Gate::Analysis => layout.plan_dir(feature).join("analysis.md"),
        Gate::Plan => layout.plan_dir(feature).join("plan.md"),
    }
}

/// SHA-256 of the gate's artifact content. `Ok(None)` when the artifact does
/// not exist — a vanished artifact is drift, not an error, until a human
/// either restores it or re-approves.
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

/// Whether `gate`'s artifact exists on disk.
///
/// An artifact that was never written is not a gate — this is the predicate
/// the whole optional-SPDD rule turns on. See [`first_blocking_upstream`].
pub(super) fn artifact_exists(
    layout: &Layout,
    feature: &FeatureName,
    gate: Gate,
) -> Result<bool, Failure> {
    Ok(fs::is_file(&artifact_path(layout, feature, gate))?)
}

/// Which SPDD artifact — the file a gate fingerprints.
///
/// Lives here beside [`artifact_path`] and [`artifact_fingerprint`], the two
/// functions that answer questions about these files. `show` re-exports it, so
/// `plan::show::Artifact` still resolves for anyone who named that path.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Artifact {
    Requirements,
    Analysis,
    Plan,
}

impl Artifact {
    /// Every artifact, in canonical (lifecycle) order — the set `plan create`
    /// scaffolds when no subset is named.
    pub const ALL: [Artifact; 3] = [Artifact::Requirements, Artifact::Analysis, Artifact::Plan];

    /// The artifact's filename.
    #[must_use]
    pub const fn filename(self) -> &'static str {
        match self {
            Self::Requirements => "requirements.md",
            Self::Analysis => "analysis.md",
            Self::Plan => "plan.md",
        }
    }

    /// The gate this artifact backs. One mapping, shared by `create` and the
    /// gate rule, so the two can never disagree about which file is which.
    #[must_use]
    pub const fn gate(self) -> Gate {
        match self {
            Self::Requirements => Gate::Requirements,
            Self::Analysis => Gate::Analysis,
            Self::Plan => Gate::Plan,
        }
    }
}
