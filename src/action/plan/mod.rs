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

use crate::domain::feature::{ApprovalState, Gate, GateState};
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

/// The first gate upstream of `gate` whose artifact exists on disk and is not
/// approved, scanning upstream-first. `None` when every upstream gate is
/// either absent or approved.
///
/// An artifact that was never written is not a gate. That is what lets a small
/// change carry a `plan.md` alone, with no `requirements.md` and no
/// `analysis.md` to approve.
///
/// The walk is deliberately **transitive**, not immediate-upstream. With
/// `requirements.md` written but unapproved and `analysis.md` absent, consulting
/// only `gate.upstream()` would wave `plan` straight through — an artifact a
/// human wrote and never approved would have stopped nothing, and the escape
/// hatch would be a `--force` in disguise. The escape is "never written", never
/// "written and ignored".
///
/// Correctness rests on `Gate::ALL` being in lifecycle order; if a gate is ever
/// inserted, re-read this loop.
pub(super) fn first_blocking_upstream(
    approvals: &ApprovalState,
    layout: &Layout,
    feature: &FeatureName,
    gate: Gate,
) -> Result<Option<Gate>, Failure> {
    for candidate in Gate::ALL {
        if candidate == gate {
            break;
        }
        if !artifact_exists(layout, feature, candidate)? {
            continue;
        }
        if approvals.state(candidate) != Some(GateState::Approved) {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

/// Every gate whose recorded approval no longer holds against the files on
/// disk right now: it is `Approved`, but some upstream artifact exists and is
/// not approved.
///
/// This is the same question [`first_blocking_upstream`] answers for a gate
/// about to be crossed, asked instead of a gate already crossed. It needs no
/// history — not "did the artifact set grow since approval?", which would mean
/// recording what existed at approval time, but "is this approval coherent
/// with the files as they are?", which is a function of the present.
///
/// Without it, writing `requirements.md` after the plan gate was approved
/// would leave `approve` refusing while `status` still reported approved and
/// `deliver` still shipped — the tool enforcing one rule and reporting another.
pub(super) fn incoherent_approvals(
    approvals: &ApprovalState,
    layout: &Layout,
    feature: &FeatureName,
) -> Result<Vec<Gate>, Failure> {
    let mut stale = Vec::new();
    for gate in Gate::ALL {
        if approvals.state(gate) != Some(GateState::Approved) {
            continue;
        }
        if first_blocking_upstream(approvals, layout, feature, gate)?.is_some() {
            stale.push(gate);
        }
    }
    Ok(stale)
}

/// `approvals`, corrected against the files as they are right now.
///
/// Two corrections, in order. **Content**: an approved gate whose artifact no
/// longer hashes to the recorded fingerprint is invalidated, cascading
/// downstream; a vanished artifact counts as changed. **Shape**: an approved
/// gate with an upstream artifact that has since been written and left
/// unapproved is invalidated too — see [`incoherent_approvals`].
///
/// Every surface that asks "is this gate approved?" has to ask it through
/// here. Reading `approvals.json` raw answers a question about the last
/// command that wrote it, not about the feature as it stands, which is how
/// `deliver` came to report a gate approved while `plan status` reported the
/// same gate as needing revision.
pub(super) fn reconciled(
    approvals: &ApprovalState,
    layout: &Layout,
    feature: &FeatureName,
) -> Result<ApprovalState, Failure> {
    let mut out = approvals.clone();
    out.normalize();

    let mut drifted = Vec::new();
    for record in &out.gates {
        if record.state != GateState::Approved {
            continue;
        }
        if artifact_fingerprint(layout, feature, record.gate)? != record.artifact_fingerprint {
            drifted.push(record.gate);
        }
    }
    for gate in drifted {
        out.invalidate_from(gate);
    }

    for gate in incoherent_approvals(&out, layout, feature)? {
        out.invalidate_from(gate);
    }

    Ok(out)
}

/// The `plan` gate's state for `feature`, fully reconciled — the read
/// `deliver` and `integrate` share with `plan approve`.
///
/// Reading `approvals.json` raw answers a question about the last command that
/// wrote it, not about the feature as it stands. That is how `deliver` came to
/// ship a plan `ivar plan status` was already reporting as needing revision.
///
/// This applies both corrections, content and shape. It can only do that
/// because `write_close` reseals the plan approval across the frontmatter it
/// stamps (see `action::feature::lifecycle`); without that, closing a feature
/// would invalidate the gate that authorised the close, and a second
/// `integrate` on a closed child would be refused.
pub(super) fn effective_plan_gate(
    layout: &Layout,
    feature: &FeatureName,
) -> Result<GateState, Failure> {
    let stored = ApprovalState::read(layout, feature)?.unwrap_or_default();
    Ok(reconciled(&stored, layout, feature)?
        .state(Gate::Plan)
        .unwrap_or(GateState::Pending))
}
