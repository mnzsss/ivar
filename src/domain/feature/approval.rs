//! The three SPDD approval gates and their state: `Gate`, `GateState`,
//! `UnknownGate`, `ApprovalState`, and one `GateRecord` per gate.
//!
//! Earlier releases had a fourth approval record for execution planning. An
//! approved Plan now authorises execution. Old `approvals.json` files may carry
//! that retired record, which `store::feature` drops at the JSON-value migration
//! layer before this enum sees it.
//!
//! Pure data, no I/O — the persisted form lives at
//! `features/<feature>/planning/approvals.json`, written by `store::feature`.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Failure, FixAction};

/// One of the three SPDD approval gates, in lifecycle order.
///
/// A gate is crossed by an explicit command after a human reviews its
/// artifact; once crossed it blocks edits to that artifact unless invalidated
/// by a change to an upstream artifact. The chain: Requirements has no
/// upstream, Analysis requires Requirements, and Plan requires Analysis. Plan
/// is the last gate — nothing downstream of it is approved separately, because
/// an approved plan is itself the authorisation a run pins its fingerprint to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    /// `requirements.md` — what the feature must do.
    Requirements,
    /// `analysis.md` — how the requirements will be met.
    Analysis,
    /// `plan.md` — the step-by-step implementation plan, and the last gate.
    Plan,
}

impl Gate {
    /// The three gates in lifecycle order, upstream first.
    pub const ALL: [Gate; 3] = [Gate::Requirements, Gate::Analysis, Gate::Plan];

    /// The gate that must be [`GateState::Approved`] before this one may be.
    /// `Requirements` is the root of the chain and has no upstream.
    #[must_use]
    pub const fn upstream(self) -> Option<Gate> {
        match self {
            Gate::Requirements => None,
            Gate::Analysis => Some(Gate::Requirements),
            Gate::Plan => Some(Gate::Analysis),
        }
    }

    /// This gate and every gate downstream of it, in lifecycle order — the set
    /// invalidated when this gate's artifact changes.
    #[must_use]
    pub const fn and_downstream(self) -> &'static [Gate] {
        match self {
            Gate::Requirements => &[Gate::Requirements, Gate::Analysis, Gate::Plan],
            Gate::Analysis => &[Gate::Analysis, Gate::Plan],
            Gate::Plan => &[Gate::Plan],
        }
    }

    /// This gate's position in [`Gate::ALL`] — how records sort into lifecycle
    /// order.
    const fn index(self) -> usize {
        match self {
            Gate::Requirements => 0,
            Gate::Analysis => 1,
            Gate::Plan => 2,
        }
    }

    /// Parse the CLI spelling of a gate name — the human-facing names, which
    /// [`fmt::Display`] emits and which serde also writes on disk now that no
    /// gate name has two words in it.
    ///
    /// `execution-graph` is deliberately *not* accepted, not even as a
    /// deprecated alias: accepting it would let a stale command or an agent
    /// working from old documentation approve something, and there is nothing
    /// left for it to approve.
    pub fn parse(value: &str) -> Result<Self, UnknownGate> {
        match value {
            "requirements" => Ok(Gate::Requirements),
            "analysis" => Ok(Gate::Analysis),
            "plan" => Ok(Gate::Plan),
            other => Err(UnknownGate(other.to_owned())),
        }
    }
}

impl fmt::Display for Gate {
    /// The human-facing, CLI spelling — the same string serde writes, now
    /// that no gate name has two words in it. Goes through
    /// [`fmt::Formatter::pad`], so width/alignment format specs (`{:<16}` in
    /// the CLI's gate table) actually pad.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Gate::Requirements => "requirements",
            Gate::Analysis => "analysis",
            Gate::Plan => "plan",
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
#[error("unknown gate `{0}` — expected one of: requirements, analysis, plan")]
pub struct UnknownGate(pub String);

impl From<UnknownGate> for Failure {
    fn from(error: UnknownGate) -> Self {
        Failure::blocked("plan.unknown_gate", error.to_string()).fix(FixAction::safe(
            "plan.valid_gate",
            "Use one of: requirements, analysis, plan.",
        ))
    }
}

/// A feature's approval state: one record per gate, the fingerprint of the
/// artifact content each approval was recorded against.
///
/// Persisted per feature at `features/<feature>/planning/approvals.json`
/// (schema v2, `Policy::Local`) by `store::feature`. `gates` always holds all
/// three after [`ApprovalState::normalize`]; a hand-edited file may omit some,
/// and the missing ones read as `Pending`. A v1 file's fourth
/// `execution_graph` record is dropped by the store's migration before this
/// type deserializes, so it never reaches [`normalize`](Self::normalize) as an
/// unknown gate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalState {
    /// One record per gate, in lifecycle order.
    pub gates: Vec<GateRecord>,
}

impl ApprovalState {
    /// A fresh state: all three gates pending, no fingerprints.
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

#[cfg(test)]
#[path = "../../../tests/unit/domain/feature/approval.rs"]
mod tests;
