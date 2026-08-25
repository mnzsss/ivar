//! `ivar plan status <plan-path>` — show the three SPDD gates and run receipt.
//!
//! The status surface reconciles approval drift without rewriting it. It may
//! import a legacy `board.json` into immutable receipt evidence; that local
//! migration is the only write it performs.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::feature::{ApprovalState, Gate, GateState, RunReceipt, RunStatus};
use crate::domain::name::FeatureName;
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;
use crate::store::feature::run;
use crate::store::layout::Layout;

use super::super::discover_hall;

#[derive(Debug, Clone)]
pub struct StatusInput {
    /// A file or directory under `plans/<feature>/`.
    pub plan_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GateStatus {
    pub gate: Gate,
    pub state: GateState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalidated_by: Option<String>,
}

/// The current receipt's relationship to the plan being inspected.
#[derive(Debug, Clone, Serialize)]
pub struct ReceiptStatus {
    pub id: String,
    pub status: RunStatus,
    pub plan_fingerprint: String,
    pub plan_matches: bool,
    pub evidence_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusOutcome {
    pub root: Utf8PathBuf,
    pub feature: FeatureName,
    pub plan_path: Utf8PathBuf,
    pub gates: Vec<GateStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<ReceiptStatus>,
    /// Whether receipt evidence exists, including archived receipts.
    pub evidence_available: bool,
}

impl WriteHuman for StatusOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "SPDD status for feature `{}` (plan: {}):",
            self.feature, self.plan_path
        )?;
        for gate in &self.gates {
            match &gate.invalidated_by {
                Some(reason) => writeln!(w, "  {:<16} {:<16} — {reason}", gate.gate, gate.state)?,
                None => writeln!(w, "  {:<16} {}", gate.gate, gate.state)?,
            }
        }
        if let Some(receipt) = &self.receipt {
            writeln!(w, "Run {}: {}", receipt.id, receipt.status)?;
            if !receipt.plan_matches {
                writeln!(w, "  plan.md changed since this run was authorised")?;
            }
            if let Some(recovery) = &receipt.recovery {
                writeln!(w, "  {recovery}")?;
            }
        } else if self.evidence_available {
            writeln!(w, "Run evidence: archived")?;
        }
        Ok(())
    }
}

pub fn status(ctx: &Ctx, input: StatusInput) -> Outcome<StatusOutcome> {
    let layout = discover_hall(ctx)?;
    let (feature, plan_path) = derive_feature(ctx, &layout, &input.plan_path)?;

    // A legacy board is execution evidence, not a status model. Import it once
    // so every consumer reads the receipt archive thereafter.
    let plan = layout.plan_dir(&feature).join("plan.md");
    let _ = run::import(
        &layout,
        &feature,
        plan,
        crate::domain::feature::RunId::new(uuid::Uuid::new_v4().to_string())?,
        &rfc3339_now(),
    )?;

    let approvals = super::load_approvals(&layout, &feature)?;
    let gates = compute_gates(&approvals, &layout, &feature)?;
    let current_fingerprint = super::artifact_fingerprint(&layout, &feature, Gate::Plan)?;
    let receipt = RunReceipt::read(&layout, &feature)?.map(|receipt| ReceiptStatus {
        id: receipt.id.to_string(),
        status: receipt.status,
        plan_fingerprint: receipt.plan_fingerprint.clone(),
        plan_matches: current_fingerprint.as_deref() == Some(receipt.plan_fingerprint.as_str()),
        evidence_available: true,
        recovery: recovery(&receipt),
    });
    let evidence_available = receipt.is_some() || !run::history(&layout, &feature)?.is_empty();

    Ok(Report::new(StatusOutcome {
        root: layout.root().to_path_buf(),
        feature,
        plan_path,
        gates,
        receipt,
        evidence_available,
    }))
}

fn recovery(receipt: &RunReceipt) -> Option<String> {
    match receipt.status {
        RunStatus::Active => Some(
            "Run is active; finish, interrupt, or resume it before starting another run."
                .to_owned(),
        ),
        RunStatus::Blocked => {
            Some("Run is blocked; resume it after resolving the coordinator's question.".to_owned())
        }
        RunStatus::Diverged => {
            Some("Run diverged from plan.md; accept the revision or interrupt the run.".to_owned())
        }
        RunStatus::Succeeded | RunStatus::Failed | RunStatus::Interrupted => None,
    }
}

fn derive_feature(
    ctx: &Ctx,
    layout: &Layout,
    plan_path: &str,
) -> Result<(FeatureName, Utf8PathBuf), Failure> {
    let resolved = ctx.resolve(Utf8Path::new(plan_path));
    let canonical = canonicalize_lenient(&resolved)?;
    let dir = if fs::is_dir(&canonical)? {
        canonical.clone()
    } else {
        canonical
            .parent()
            .map(Utf8Path::to_path_buf)
            .ok_or_else(|| not_a_plan(&resolved, layout))?
    };
    let plans_dir = canonicalize_lenient(&layout.root().join("plans"))?;
    if dir.parent() != Some(plans_dir.as_path()) {
        return Err(not_a_plan(&resolved, layout));
    }
    let Some(raw_name) = dir.file_name() else {
        return Err(not_a_plan(&resolved, layout));
    };
    Ok((
        FeatureName::new(raw_name).map_err(|_| not_a_plan(&resolved, layout))?,
        resolved,
    ))
}

fn canonicalize_lenient(path: &Utf8Path) -> Result<Utf8PathBuf, Failure> {
    let mut existing = path;
    let mut tail = Vec::new();
    while !fs::exists(existing)? {
        let Some(name) = existing.file_name() else {
            break;
        };
        tail.push(name);
        let Some(parent) = existing.parent() else {
            break;
        };
        existing = parent;
    }
    let mut canonical = std::fs::canonicalize(existing)
        .map_err(|source| Failure::blocked("plan.status_path_unreadable", source.to_string()))?;
    for name in tail.iter().rev() {
        canonical.push(name);
    }
    Utf8PathBuf::from_path_buf(canonical).map_err(|path| {
        Failure::blocked(
            "plan.status_path_not_utf8",
            format!("plan path is not valid UTF-8: {}", path.display()),
        )
    })
}

fn not_a_plan(path: &Utf8Path, layout: &Layout) -> Failure {
    Failure::blocked(
        "plan.status_not_a_plan",
        format!("`{path}` is not a feature plan path"),
    )
    .expected(format!(
        "a file or directory under `{}/plans/<feature>/`",
        layout.root()
    ))
    .actual("the path does not sit under the hall's plans directory for a feature")
    .fix(FixAction::safe(
        "plan.status_pass_plan_path",
        "Pass the plan path relative to the hall root, e.g. `plans/checkout/plan.md`.",
    ))
}

/// The gates a feature actually has, in lifecycle order, with drift already
/// reconciled.
///
/// A gate is omitted from the result when its artifact does not exist on disk
/// **and** its stored state is [`GateState::Pending`] — the feature
/// deliberately never wrote that artifact, so it was never a gate to begin
/// with (R-STATUS-OMITS). The conjunction is load-bearing: an absent artifact
/// whose gate was `Approved` (or `NeedsRevision`) is *not* omitted, and keeps
/// flowing through the drift branch below to `needs-revision` with its
/// `invalidated_by` reason (R-STATUS-DRIFT). Omitting on absence alone would
/// silently clear a gate whose approved artifact someone deleted, turning
/// this rule into a hole instead of a fix. An omitted gate also never sets
/// `previous_invalidated`, so it cannot seed a downstream cascade — but the
/// cascade still passes through it untouched, since skipping it never resets
/// whatever an earlier gate already set.
fn compute_gates(
    approvals: &ApprovalState,
    layout: &Layout,
    feature: &FeatureName,
) -> Result<Vec<GateStatus>, Failure> {
    let mut drift_roots = Vec::new();
    for record in &approvals.gates {
        if record.state == GateState::Approved
            && super::artifact_fingerprint(layout, feature, record.gate)?
                != record.artifact_fingerprint
        {
            drift_roots.push(record.gate);
        }
    }
    let stale = super::incoherent_approvals(approvals, layout, feature)?;

    let mut gates = Vec::new();
    let mut previous_invalidated = None;
    for gate in Gate::ALL {
        let stored = approvals
            .record(gate)
            .map_or(GateState::Pending, |record| record.state);
        if stored == GateState::Pending && !super::artifact_exists(layout, feature, gate)? {
            // Never crossed and nothing on disk — not a gate for this feature.
            continue;
        }
        let (state, invalidated_by) = if drift_roots.contains(&gate) {
            (
                GateState::NeedsRevision,
                Some(format!(
                    "`{}` changed since approval",
                    super::artifact_path(layout, feature, gate)
                )),
            )
        } else if stale.contains(&gate) {
            let blocking = super::first_blocking_upstream(approvals, layout, feature, gate)?
                .map_or_else(String::new, |upstream| upstream.to_string());
            (
                GateState::NeedsRevision,
                Some(format!("`{blocking}` was written after this was approved")),
            )
        } else if let Some(upstream) = previous_invalidated {
            (
                GateState::NeedsRevision,
                Some(format!("cascaded from `{upstream}`")),
            )
        } else if stored == GateState::NeedsRevision {
            (stored, Some("explicitly invalidated".to_owned()))
        } else {
            (stored, None)
        };
        if state == GateState::NeedsRevision {
            previous_invalidated = Some(gate);
        }
        gates.push(GateStatus {
            gate,
            state,
            invalidated_by,
        });
    }
    Ok(gates)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/plan/status.rs"]
mod tests;
