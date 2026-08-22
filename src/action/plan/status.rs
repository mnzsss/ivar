//! `ivar plan status <plan-path>` — read the SPDD cycle state for one
//! feature: the four approval gates, what invalidated each, and the
//! execution board.
//!
//! This is the read surface the approval state had been missing: nobody but
//! `plan approve` / `plan invalidate` ever read `approvals.json`, so a gate
//! could drift — or the board and the `execution-graph` gate could diverge —
//! with no command to see it. This verb exists to make that impossible to
//! miss.
//!
//! # What invalidated each gate
//!
//! The state is *computed*, not just echoed from disk. An approved gate whose
//! artifact no longer matches its stored fingerprint is shown as
//! `needs-revision` naming the artifact that changed; everything downstream
//! of a changed gate is shown as `needs-revision` cascaded from it. The
//! computation is read-only — this verb never writes, so an honest look never
//! becomes a hidden repair.
//!
//! # Board × gate divergence
//!
//! The `execution-graph` gate is shown next to the board's status, and the
//! two are checked against each other: a board in `Approved` requires the
//! gate to be `Approved` (the predecessor's TS let a board sit `approved`
//! while the gate stayed `draft` — the bug this surface exists to surface).
//! When they disagree, [`StatusOutcome::divergent`] is set and the human
//! surface says so loudly, naming `ivar feature execute approve` as the way
//! out.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::feature::{ApprovalState, ExecutionBoard, ExecutionStatus, Gate, GateState};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::{fs, hash};
use crate::store::feature;
use crate::store::layout::Layout;

use super::super::discover_hall;
use crate::action::Ctx;

/// What `ivar plan status` needs.
#[derive(Debug, Clone)]
pub struct StatusInput {
    /// Path to the feature's plan — a file under `plans/<feature>/`
    /// (`plan.md`, `requirements.md`, …) or the plan directory itself,
    /// relative to the current directory.
    pub plan_path: String,
}

/// One gate's displayed state.
#[derive(Debug, Clone, Serialize)]
pub struct GateStatus {
    /// The gate.
    pub gate: Gate,
    /// The gate's state, drift-checked against its artifact.
    pub state: GateState,
    /// What invalidated the gate, when it is shown as `needs-revision`: the
    /// artifact that changed, the gate it cascaded from, or the explicit
    /// `plan invalidate`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalidated_by: Option<String>,
}

/// The feature's execution board, when one exists.
#[derive(Debug, Clone, Serialize)]
pub struct BoardStatus {
    /// Where the board lives.
    pub board_path: Utf8PathBuf,
    /// The board's overall status.
    pub status: ExecutionStatus,
    /// How many workstreams the board's graph declares.
    pub workstreams: usize,
    /// The plan.md fingerprint the board was prepared from.
    pub plan_fingerprint: String,
    /// Whether that fingerprint still matches plan.md — `false` when the
    /// plan changed after the board was prepared, which voids the graph.
    pub plan_matches: bool,
}

/// What `ivar plan status` found.
#[derive(Debug, Clone, Serialize)]
pub struct StatusOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the plan path resolved to.
    pub feature: FeatureName,
    /// The plan path as resolved against the current directory.
    pub plan_path: Utf8PathBuf,
    /// The four gates, in lifecycle order, each with what invalidated it.
    pub gates: Vec<GateStatus>,
    /// The execution board, when the feature has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board: Option<BoardStatus>,
    /// Whether the `execution-graph` gate disagrees with the board: a board
    /// in `Approved` requires the gate to be `Approved`. `true` is exactly
    /// the divergence the predecessor's TS permitted; this surface exists to
    /// make it visible.
    pub divergent: bool,
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
        if let Some(board) = &self.board {
            let noun = if board.workstreams == 1 {
                "workstream"
            } else {
                "workstreams"
            };
            writeln!(
                w,
                "Board: {} ({} {noun}) — {}",
                board.status, board.workstreams, board.board_path
            )?;
            if !board.plan_matches {
                writeln!(
                    w,
                    "  plan.md changed since the board was prepared — the graph is void"
                )?;
            }
        }
        if self.divergent {
            writeln!(
                w,
                "DIVERGENCE: the board is `approved` but the `execution-graph` gate is not — \
                 run `ivar feature execute approve`."
            )?;
        }
        Ok(())
    }
}

/// Show the SPDD cycle state for the feature `input.plan_path` names.
///
/// The feature is derived from the plan path — it must sit under
/// `<hall>/plans/<feature>/` — and the approvals and board are read for that
/// feature. Read-only: nothing here is written, whatever the drift.
pub fn status(ctx: &Ctx, input: StatusInput) -> Outcome<StatusOutcome> {
    let layout = discover_hall(ctx)?;
    let (feature, plan_path) = derive_feature(ctx, &layout, &input.plan_path)?;

    let approvals = super::load_approvals(&layout, &feature)?;
    let gates = compute_gates(&approvals, &layout, &feature)?;

    let board = match ExecutionBoard::read(&layout, &feature)? {
        Some(board) => Some(BoardStatus {
            board_path: feature::board_path(&layout, &feature),
            status: board.status,
            workstreams: board.graph.workstreams.len(),
            plan_fingerprint: board.graph.plan_fingerprint.clone(),
            plan_matches: plan_matches(&layout, &feature, &board)?,
        }),
        None => None,
    };

    let divergent = false;

    Ok(Report::new(StatusOutcome {
        root: layout.root().to_path_buf(),
        feature,
        plan_path,
        gates,
        board,
        divergent,
    }))
}

/// Resolve `plan_path` to the feature it names: a file or directory under
/// `<hall>/plans/<feature>`. Anything else is blocked, so a typo cannot
/// silently read another feature's gates.
///
/// The path is canonicalised first — through the deepest existing ancestor,
/// see [`canonicalize_lenient`] — so a plan path projected through a session
/// view dir's `plans/<feature>` symlink is accepted: the agent inside a
/// session runs `ivar plan status plans/<feature>/plan.md` against the
/// session's own view dir, and the path resolves to the hall's real plan
/// directory. The same canonicalisation is what keeps a symlink that escapes
/// the hall's plan directory refused.
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
    let feature = FeatureName::new(raw_name).map_err(|_| not_a_plan(&resolved, layout))?;

    Ok((feature, resolved))
}

/// The canonical, symlink-free form of `path`, tolerating a nonexistent tail.
///
/// `std::fs::canonicalize` refuses any path with a missing component. Plan
/// status must still work on a plan file that was never written, and on a
/// plan directory whose artifacts do not exist yet — and it must accept a
/// plan path projected through a view dir's `plans/<feature>` symlink whose
/// target directory may itself not exist. So the deepest *existing* ancestor
/// is canonicalised, then the remaining components are appended back —
/// resolving each through `readlink`, so a dangling symlink lands on the
/// directory it points at rather than staying unresolved.
fn canonicalize_lenient(path: &Utf8Path) -> Result<Utf8PathBuf, Failure> {
    canonicalize_lenient_depth(path, 0)
}

/// The depth-limited worker behind [`canonicalize_lenient`]. `depth` bounds
/// how many symlink hops one call may take, so a symlink cycle fails instead
/// of recursing forever.
fn canonicalize_lenient_depth(path: &Utf8Path, depth: u32) -> Result<Utf8PathBuf, Failure> {
    if depth > 16 {
        return Err(Failure::failed(
            "fs.symlink_loop",
            format!("too many symlink hops while resolving `{path}`"),
        ));
    }

    // The deepest ancestor that exists, and the components below it.
    let mut suffix: Vec<String> = Vec::new();
    let mut current = path;
    while !fs::exists(current)? {
        let Some(name) = current.file_name() else {
            return Ok(current.to_path_buf());
        };
        let Some(parent) = current.parent() else {
            return Ok(current.to_path_buf());
        };
        suffix.push(name.to_owned());
        current = parent;
    }

    let mut canonical = current.canonicalize_utf8().map_err(|source| {
        Failure::failed(
            "fs.canonicalize_failed",
            format!("could not resolve `{current}`: {source}"),
        )
    })?;

    // `suffix` holds the components below the existing ancestor, leaf-first
    // (the climb pushed from the leaf up). Flip it so components run root-ward
    // to leaf-ward, then append each, resolving symlinks through `readlink`.
    let mut components: Vec<String> = suffix;
    components.reverse();
    for (index, name) in components.iter().enumerate() {
        let candidate = canonical.join(name);
        if let fs::SymlinkTarget::Target(target) = fs::read_symlink(&candidate)? {
            // The target is relative to the symlink's parent; everything
            // still to come is re-appended after it. Then re-derive from the
            // target: its own ancestors may be symlinks too, and it may not
            // exist yet either.
            let mut combined = if target.is_absolute() {
                target
            } else {
                canonical.join(&target)
            };
            for remaining in components.iter().skip(index + 1) {
                combined.push(remaining);
            }
            return canonicalize_lenient_depth(&combined, depth + 1);
        }
        canonical = candidate;
    }
    Ok(canonical)
}

/// The blocked failure for a plan path that is not a plan of this hall.
fn not_a_plan(resolved: &Utf8Path, layout: &Layout) -> Failure {
    Failure::blocked(
        "plan.status_not_a_plan",
        format!("`{resolved}` is not an SPDD plan of this hall"),
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

/// The gates, drift-checked against their artifacts, with what invalidated
/// each. Read-only: the stored state is never rewritten, only reported.
fn compute_gates(
    approvals: &ApprovalState,
    layout: &Layout,
    feature: &FeatureName,
) -> Result<Vec<GateStatus>, Failure> {
    // Drift roots: approved gates whose artifact no longer matches the
    // fingerprint their approval was recorded against.
    let mut drift_roots = Vec::new();
    for record in &approvals.gates {
        if record.state == GateState::Approved
            && super::artifact_fingerprint(layout, feature, record.gate)?
                != record.artifact_fingerprint
        {
            drift_roots.push(record.gate);
        }
    }

    let mut gates = Vec::new();
    // The nearest upstream gate shown as needs-revision — the cascade parent.
    let mut previous_invalidated: Option<Gate> = None;
    for gate in Gate::ALL {
        let stored = approvals
            .record(gate)
            .map_or(GateState::Pending, |record| record.state);

        let (state, invalidated_by) = if drift_roots.contains(&gate) {
            (
                GateState::NeedsRevision,
                Some(format!(
                    "`{}` changed since approval",
                    artifact_name(layout, feature, gate)
                )),
            )
        } else if let Some(upstream) = previous_invalidated {
            (
                GateState::NeedsRevision,
                Some(format!("cascaded from `{upstream}`")),
            )
        } else {
            match stored {
                GateState::NeedsRevision => (
                    GateState::NeedsRevision,
                    Some("invalidated by `ivar plan invalidate`".to_owned()),
                ),
                other => (other, None),
            }
        };

        previous_invalidated = if state == GateState::NeedsRevision {
            Some(gate)
        } else {
            None
        };
        gates.push(GateStatus {
            gate,
            state,
            invalidated_by,
        });
    }
    Ok(gates)
}

/// Whether the board's plan fingerprint still matches plan.md — `false` when
/// the plan changed after preparation, which voids the graph.
fn plan_matches(
    layout: &Layout,
    feature: &FeatureName,
    board: &ExecutionBoard,
) -> Result<bool, Failure> {
    let plan = layout.plan_dir(feature).join("plan.md");
    if !fs::is_file(&plan)? {
        return Ok(false);
    }
    Ok(hash::file(&plan)? == board.graph.plan_fingerprint)
}

/// The artifact's path relative to the hall root, for a human reason string.
fn artifact_name(layout: &Layout, feature: &FeatureName, gate: Gate) -> String {
    let path = super::artifact_path(layout, feature, gate);
    path.strip_prefix(layout.root())
        .map(|relative| relative.to_string())
        .unwrap_or_else(|_| path.to_string())
}

// Board-status tests were retired with the execution-graph gate.
