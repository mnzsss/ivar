//! `ivar plan approve <gate>` and `ivar plan invalidate <gate>` — the four
//! approval gates.
//!
//! # The gates
//!
//! Each feature's SPDD lifecycle has four gates — Requirements, Analysis,
//! Plan, and Execution Graph — crossed in that order. A gate is crossed by an
//! explicit `plan approve`, which records a SHA-256 fingerprint of the
//! artifact's content. Crossing a gate requires the gate immediately upstream
//! to be approved first; Requirements is the root of the chain and is always
//! approvable.
//!
//! # Invalidation
//!
//! An approval is void the moment its artifact's content changes — the stored
//! fingerprint no longer matches the file. `approve` reconciles the stored
//! state against the current files before doing anything else, and every
//! drift it finds invalidates that gate **and everything downstream of it**:
//! changing `requirements.md` sets Analysis, Plan, and Execution Graph to
//! `NeedsRevision`. This is the CONTEXT.md contract — "a gate once crossed
//! blocks edits to its artifact unless the gate is invalidated by a change to
//! an upstream artifact". The gates cannot stop a human from editing Markdown;
//! what they do is refuse to keep treating the artifact as approved once its
//! content has moved. The reconciliation is persisted even when the approval
//! itself is then refused, so an invalidated state is never left on disk
//! pretending an approval still stands.
//!
//! `plan invalidate` is the explicit half of the same rule: the human
//! declaring "I am revising this artifact" before the edit lands. It marks the
//! gate and everything downstream `NeedsRevision` and clears their
//! fingerprints, without requiring a content change first. It is idempotent —
//! invalidating an already-invalidated gate changes nothing. Drift detection
//! is `approve`'s job; `invalidate` never re-reads the artifacts.
//!
//! # Artifacts
//!
//! Requirements, Analysis, and Plan fingerprint their own committed Markdown
//! file under `plans/<feature>/`. The Execution Graph has no file of its own —
//! it is derived from `plan.md`'s Operations section — so it fingerprints
//! `plan.md` too: any plan change invalidates the graph, which is exactly the
//! cascade CONTEXT.md requires.
//!
//! # Persistence
//!
//! The state lives at `features/<feature>/planning/approvals.json` (schema
//! v1, `Policy::Local` — gitignored, per-machine). An absent file means no
//! gate has ever been crossed.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{ApprovalState, Gate, GateState};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::{fs, hash};
use crate::store::layout::Layout;

use super::super::discover_hall;
use crate::action::Ctx;

/// What `ivar plan approve` needs.
#[derive(Debug, Clone)]
pub struct ApproveInput {
    /// The feature whose gate to approve.
    pub feature: String,
    /// The gate to approve, as [`Gate::parse`] understands it.
    pub gate: String,
}

/// What `ivar plan approve` did.
#[derive(Debug, Clone, Serialize)]
pub struct ApproveOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature.
    pub feature: FeatureName,
    /// The gate that was approved.
    pub gate: Gate,
    /// The full approval state after the transition — the gate itself, and
    /// any gates the reconciliation just invalidated.
    pub approvals: ApprovalState,
}

impl WriteHuman for ApproveOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Approved `{}` for feature `{}`", self.gate, self.feature)?;
        write_gates(w, &self.approvals)
    }
}

/// What `ivar plan invalidate` needs.
#[derive(Debug, Clone)]
pub struct InvalidateInput {
    /// The feature whose gate to invalidate.
    pub feature: String,
    /// The gate under revision, as [`Gate::parse`] understands it.
    pub gate: String,
}

/// What `ivar plan invalidate` did.
#[derive(Debug, Clone, Serialize)]
pub struct InvalidateOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature.
    pub feature: FeatureName,
    /// The gate the human declared under revision.
    pub gate: Gate,
    /// The gates this run actually flipped to `NeedsRevision` — the gate and
    /// its downstream, minus any already there. Empty when the run was a
    /// no-op.
    pub cascaded: Vec<Gate>,
    /// The full approval state after the transition.
    pub approvals: ApprovalState,
}

impl WriteHuman for InvalidateOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Invalidated `{}` for feature `{}`",
            self.gate, self.feature
        )?;
        write_gates(w, &self.approvals)
    }
}

/// Cross `gate` for `input.feature`, recording the artifact's current content
/// fingerprint.
///
/// Blocked when the feature does not exist, the gate name is unknown, the
/// gate's artifact has not been written, or the gate upstream of it is not
/// approved. Before any of that is checked, the stored state is reconciled
/// against the current files — an approved gate whose artifact changed is
/// invalidated, cascading downstream — so an approval never stands on content
/// that has moved.
pub fn approve(ctx: &Ctx, input: ApproveInput) -> Outcome<ApproveOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;
    let gate = Gate::parse(&input.gate)?;

    // The execution-graph gate has exactly one writer: `ivar feature execute
    // approve`. `plan approve` refusing it here is what prevents the
    // board/gate divergence the predecessor TS shipped — a gate with two
    // approvers (one of which was unreachable) is precisely how that bug was
    // born.
    if gate == Gate::ExecutionGraph {
        return Err(Failure::blocked(
            "plan.approve_execution_graph_via_execute",
            "the `execution-graph` gate is approved by `ivar feature execute approve`, not `plan approve`",
        )
        .expected("approving the execution graph through the execute path")
        .actual("`plan approve` cannot write the execution-graph gate")
        .fix(FixAction::safe(
            "execute.approve",
            "Run `ivar feature execute approve <feature>` to approve the execution graph.",
        )
        .command("ivar feature execute approve")));
    }

    require_feature(&layout, &feature)?;

    let feature_record =
        crate::domain::feature::Feature::read(&layout, &feature)?.ok_or_else(|| {
            Failure::blocked(
                "plan.feature_vanished",
                format!("feature `{feature}` has a directory but no feature.json"),
            )
        })?;
    crate::action::feature::ensure_not_fully_integrated(&layout, &feature_record)?;

    let mut approvals = load_approvals(&layout, &feature)?;

    // Drift first — and persist it before any refusal below: an approval must
    // never stand on content that has moved, and a refused approval still has
    // to leave the recorded state honest for the next inspection.
    let drifted = reconcile(&mut approvals, &layout, &feature)?;
    if drifted {
        approvals.write(&layout, &feature)?;
    }

    let fingerprint = artifact_fingerprint(&layout, &feature, gate)?.ok_or_else(|| {
        let path = artifact_path(&layout, &feature, gate);
        Failure::blocked(
            "plan.artifact_missing",
            format!("`{}` does not exist", path),
        )
        .expected("the gate's SPDD artifact to have been written")
        .actual(format!("no artifact for the `{gate}` gate"))
        .fix(FixAction::safe(
            "plan.create_first",
            format!(
                "Write the artifact, then approve again. `ivar plan create {feature}` scaffolds it."
            ),
        ))
    })?;

    if let Some(upstream) = gate.upstream()
        && !approvals.upstream_approved(gate)
    {
        return Err(Failure::blocked(
            "plan.upstream_not_approved",
            format!(
                "cannot approve `{gate}` for `{feature}`: the upstream gate `{upstream}` is not approved"
            ),
        )
        .expected(format!("the upstream gate `{upstream}` to be approved first"))
        .actual(format!(
            "`{upstream}` is {}",
            approvals.state(upstream).unwrap_or(GateState::Pending)
        ))
        .fix(FixAction::safe(
            "plan.approve_upstream",
            format!("Approve the upstream gate first: `ivar plan approve {feature} {upstream}`."),
        )));
    }

    approvals.set(gate, GateState::Approved, Some(fingerprint));
    approvals.write(&layout, &feature)?;

    Ok(Report::new(ApproveOutcome {
        root: layout.root().to_path_buf(),
        feature,
        gate,
        approvals,
    }))
}

/// Declare `gate` under revision for `input.feature`: mark it and everything
/// downstream `NeedsRevision` and clear their fingerprints.
///
/// Unlike [`approve`], this never reads the artifacts — it is the explicit
/// "I am revising this" declaration, so it works even when the artifact is
/// unchanged or missing. Idempotent: re-running on the same state changes
/// nothing and reports an empty [`InvalidateOutcome::cascaded`].
pub fn invalidate(ctx: &Ctx, input: InvalidateInput) -> Outcome<InvalidateOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;
    let gate = Gate::parse(&input.gate)?;

    require_feature(&layout, &feature)?;

    let feature_record =
        crate::domain::feature::Feature::read(&layout, &feature)?.ok_or_else(|| {
            Failure::blocked(
                "plan.feature_vanished",
                format!("feature `{feature}` has a directory but no feature.json"),
            )
        })?;
    crate::action::feature::ensure_not_fully_integrated(&layout, &feature_record)?;

    let mut approvals = load_approvals(&layout, &feature)?;

    let already_needs_revision: Vec<Gate> = approvals
        .gates
        .iter()
        .filter(|record| record.state == GateState::NeedsRevision)
        .map(|record| record.gate)
        .collect();

    approvals.invalidate_from(gate);

    let cascaded: Vec<Gate> = gate
        .and_downstream()
        .iter()
        .copied()
        .filter(|downstream| !already_needs_revision.contains(downstream))
        .collect();

    approvals.write(&layout, &feature)?;

    Ok(Report::new(InvalidateOutcome {
        root: layout.root().to_path_buf(),
        feature,
        gate,
        cascaded,
        approvals,
    }))
}

/// Load the feature's approval state, or a fresh one if none was ever
/// written, with all four gates present and in lifecycle order.
fn load_approvals(layout: &Layout, feature: &FeatureName) -> Result<ApprovalState, Failure> {
    let mut approvals = ApprovalState::read(layout, feature)?.unwrap_or_else(ApprovalState::fresh);
    approvals.normalize();
    Ok(approvals)
}

/// Block when the feature does not exist — approvals belong to features.
fn require_feature(layout: &Layout, feature: &FeatureName) -> Result<(), Failure> {
    if fs::is_dir(&layout.feature_dir(feature))? {
        return Ok(());
    }
    Err(Failure::blocked(
        "plan.feature_not_found",
        format!("feature `{feature}` does not exist"),
    )
    .expected("an existing feature to approve gates for")
    .actual(format!("`{feature}` has no feature directory"))
    .fix(FixAction::safe(
        "feature.create_first",
        format!("Create the feature first with `ivar feature create {feature}`."),
    )))
}

/// Re-check every approved gate's fingerprint against its artifact's current
/// content. Every gate whose fingerprint no longer matches is invalidated,
/// cascading downstream. An artifact that has vanished counts as changed. Only
/// an unreadable file is an error — drift itself never is. Returns whether any
/// gate was invalidated, so the caller can persist the corrected state even
/// when the operation it is part of is about to be refused.
fn reconcile(
    approvals: &mut ApprovalState,
    layout: &Layout,
    feature: &FeatureName,
) -> Result<bool, Failure> {
    let mut drifted = Vec::new();
    for record in &approvals.gates {
        if record.state != GateState::Approved {
            continue;
        }
        let current = artifact_fingerprint(layout, feature, record.gate)?;
        if current != record.artifact_fingerprint {
            drifted.push(record.gate);
        }
    }
    for gate in &drifted {
        approvals.invalidate_from(*gate);
    }
    Ok(!drifted.is_empty())
}

/// The artifact a gate fingerprints. Requirements, Analysis, and Plan each
/// own their Markdown file; the Execution Graph is derived from `plan.md`, so
/// it fingerprints that too.
fn artifact_path(layout: &Layout, feature: &FeatureName, gate: Gate) -> Utf8PathBuf {
    match gate {
        Gate::Requirements => layout.plan_dir(feature).join("requirements.md"),
        Gate::Analysis => layout.plan_dir(feature).join("analysis.md"),
        Gate::Plan | Gate::ExecutionGraph => layout.plan_dir(feature).join("plan.md"),
    }
}

/// SHA-256 of the gate's artifact content. `Ok(None)` when the artifact does
/// not exist — a vanished artifact is drift, not an error, until a human
/// either restores it or re-approves.
fn artifact_fingerprint(
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

/// The one gate-state rendering, shared by both outcomes: one line per gate,
/// lifecycle order, states padded into a column.
fn write_gates(w: &mut impl io::Write, approvals: &ApprovalState) -> io::Result<()> {
    for record in &approvals.gates {
        writeln!(w, "  {:<16} {}", record.gate, record.state)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/unit/action/plan/approve.rs"]
mod tests;
