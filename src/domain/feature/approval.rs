//! The four SPDD approval gates and their state: `Gate`, `GateState`,
//! `UnknownGate`, `ApprovalState`, and one `GateRecord` per gate.
//!
//! Pure data, no I/O — the persisted form lives at
//! `features/<feature>/planning/approvals.json`, written by `store::feature`.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Failure, FixAction};

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
