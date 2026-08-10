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

    let approvals = load_approvals(&layout, &feature)?;
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

    let gate_state = gates
        .iter()
        .find(|gate| gate.gate == Gate::ExecutionGraph)
        .map_or(GateState::Pending, |gate| gate.state);
    let divergent = board
        .as_ref()
        .is_some_and(|board| board.status == ExecutionStatus::Approved)
        && gate_state != GateState::Approved;

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
/// `<hall>/plans/<feature>/`. Anything else is blocked, so a typo cannot
/// silently read another feature's gates.
fn derive_feature(
    ctx: &Ctx,
    layout: &Layout,
    plan_path: &str,
) -> Result<(FeatureName, Utf8PathBuf), Failure> {
    let resolved = ctx.resolve(Utf8Path::new(plan_path));
    let dir = if fs::is_dir(&resolved)? {
        resolved.clone()
    } else {
        resolved
            .parent()
            .map(Utf8Path::to_path_buf)
            .ok_or_else(|| not_a_plan(&resolved, layout))?
    };

    let plans_dir = layout.root().join("plans");
    if dir.parent() != Some(plans_dir.as_path()) {
        return Err(not_a_plan(&resolved, layout));
    }
    let Some(raw_name) = dir.file_name() else {
        return Err(not_a_plan(&resolved, layout));
    };
    let feature = FeatureName::new(raw_name).map_err(|_| not_a_plan(&resolved, layout))?;

    Ok((feature, resolved))
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

/// The feature's approval state, or a fresh one with all four gates pending,
/// normalised to lifecycle order.
fn load_approvals(layout: &Layout, feature: &FeatureName) -> Result<ApprovalState, Failure> {
    let mut approvals = ApprovalState::read(layout, feature)?.unwrap_or_else(ApprovalState::fresh);
    approvals.normalize();
    Ok(approvals)
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
            && artifact_fingerprint(layout, feature, record.gate)? != record.artifact_fingerprint
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

/// The artifact a gate fingerprints. Requirements, Analysis, and Plan each
/// own their Markdown file; the Execution Graph is derived from `plan.md`, so
/// it fingerprints that too — the same content the board's graph fingerprints.
fn artifact_path(layout: &Layout, feature: &FeatureName, gate: Gate) -> Utf8PathBuf {
    match gate {
        Gate::Requirements => layout.plan_dir(feature).join("requirements.md"),
        Gate::Analysis => layout.plan_dir(feature).join("analysis.md"),
        Gate::Plan | Gate::ExecutionGraph => layout.plan_dir(feature).join("plan.md"),
    }
}

/// SHA-256 of the gate's artifact content. `Ok(None)` when the artifact does
/// not exist — a vanished artifact is drift, not an error.
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

/// The artifact's path relative to the hall root, for a human reason string.
fn artifact_name(layout: &Layout, feature: &FeatureName, gate: Gate) -> String {
    let path = artifact_path(layout, feature, gate);
    path.strip_prefix(layout.root())
        .map(|relative| relative.to_string())
        .unwrap_or_else(|_| path.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::action::execute::approve as execute_approve;
    use crate::action::execute::prepare as execute_prepare;
    use crate::action::feature::create::{
        self as feature_create, CreateInput as FeatureCreateInput,
    };
    use crate::action::hall::{self, InitInput};
    use crate::action::plan::approve::{self as plan_approve, ApproveInput};
    use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
    use crate::error::Status;
    use crate::test_support::hall_root;

    const GRAPH_JSON: &str = r#"{
        "workstreams": [
            {
                "id": "ws-gates",
                "title": "Approval gates",
                "operations": ["add-gate-types", "wire-approve"],
                "depends_on": [],
                "write_contract": ["src/domain/feature.rs"]
            }
        ]
    }"#;

    fn seeded_hall() -> (tempfile::TempDir, Utf8PathBuf) {
        let (guard, root) = hall_root();
        let ctx = Ctx::new(root.clone());
        hall::init(
            &ctx,
            InitInput {
                path: Utf8PathBuf::from("."),
                name: Some("acme".to_owned()),
                provider: None,
            },
        )
        .unwrap();
        feature_create::create(
            &ctx,
            FeatureCreateInput {
                name: "checkout".to_owned(),
                branch: None,
            },
        )
        .unwrap();
        plan_create::create(
            &ctx,
            PlanCreateInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();
        (guard, root)
    }

    fn status_input(path: &str) -> StatusInput {
        StatusInput {
            plan_path: path.to_owned(),
        }
    }

    fn approve_gate(ctx: &Ctx, gate: &str) {
        plan_approve::approve(
            ctx,
            ApproveInput {
                feature: "checkout".to_owned(),
                gate: gate.to_owned(),
            },
        )
        .unwrap();
    }

    /// Put a freshly prepared board into the state `execute approve` demands.
    ///
    /// `prepare` currently stamps the board `Pending`; `execute approve`
    /// requires `AwaitingApproval`. The parallel OP-EXEC-* workstream owns
    /// that transition — until it lands, this pins the precondition so the
    /// real approve path (the only writer of the `execution-graph` gate) can
    /// run end to end.
    fn awaiting_approval(root: &Utf8PathBuf) {
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
        board.set_status(ExecutionStatus::AwaitingApproval);
        board.write(&layout, &feature).unwrap();
    }

    fn gate(outcome: &StatusOutcome, gate: Gate) -> &GateStatus {
        outcome.gates.iter().find(|g| g.gate == gate).unwrap()
    }

    #[test]
    fn status_shows_all_four_gates_pending_in_a_fresh_hall() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root);

        let report = status(&ctx, status_input("plans/checkout/plan.md")).unwrap();

        assert!(report.is_clean());
        assert_eq!(report.value.feature.as_str(), "checkout");
        assert_eq!(report.value.gates.len(), 4);
        for g in &report.value.gates {
            assert_eq!(g.state, GateState::Pending);
            assert!(g.invalidated_by.is_none());
        }
        assert!(report.value.board.is_none());
        assert!(!report.value.divergent);
    }

    /// The heart of the read surface: four gates, each with what invalidated
    /// it. An edited `requirements.md` invalidates requirements by drift and
    /// cascades to everything downstream.
    #[test]
    fn status_shows_the_four_gates_and_what_invalidated_each() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        for gate in ["requirements", "analysis", "plan"] {
            approve_gate(&ctx, gate);
        }

        // Edit requirements.md behind ivar's back.
        fs::write_text(
            &root.join("plans/checkout/requirements.md"),
            "# Requirements\n\n- [x] changed\n",
        )
        .unwrap();

        let report = status(&ctx, status_input("plans/checkout/plan.md")).unwrap();
        let gates = &report.value.gates;

        assert_eq!(
            gate(&report.value, Gate::Requirements).state,
            GateState::NeedsRevision
        );
        let reason = gate(&report.value, Gate::Requirements)
            .invalidated_by
            .as_deref()
            .expect("the drift reason must be named");
        assert!(
            reason.contains("requirements.md") && reason.contains("changed since approval"),
            "the drift reason must name the changed artifact: {reason}"
        );
        for (downstream, cascaded_from) in [
            (Gate::Analysis, "requirements"),
            (Gate::Plan, "analysis"),
            (Gate::ExecutionGraph, "plan"),
        ] {
            assert_eq!(
                gate(&report.value, downstream).state,
                GateState::NeedsRevision
            );
            let expected = format!("cascaded from `{cascaded_from}`");
            assert_eq!(
                gate(&report.value, downstream).invalidated_by.as_deref(),
                Some(expected.as_str())
            );
        }
        assert_eq!(gates.len(), 4);
    }

    /// Drift is only reported, never persisted: a status run must not repair
    /// the approvals file behind the human's back.
    #[test]
    fn status_does_not_write_the_drift_it_finds() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        approve_gate(&ctx, "requirements");
        fs::write_text(
            &root.join("plans/checkout/requirements.md"),
            "# Requirements\n\n- [x] changed\n",
        )
        .unwrap();

        status(&ctx, status_input("plans/checkout/plan.md")).unwrap();

        // The stored state still says approved — status read it and left it.
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        let on_disk = ApprovalState::read(&layout, &feature).unwrap().unwrap();
        assert_eq!(on_disk.state(Gate::Requirements), Some(GateState::Approved));
    }

    /// The board is shown next to the `execution-graph` gate, and after the
    /// real approve flow the two agree: board `approved`, gate `approved`,
    /// not divergent. This is the test that the two never diverge.
    #[test]
    fn status_shows_the_board_with_the_gate_and_they_never_diverge() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        let graph = root.join("graph.json");
        fs::write_text(&graph, GRAPH_JSON).unwrap();
        execute_prepare::prepare(
            &ctx,
            execute_prepare::PrepareInput {
                feature: "checkout".to_owned(),
                graph_json: graph.to_string(),
            },
        )
        .unwrap();
        awaiting_approval(&root);
        execute_approve::approve(
            &ctx,
            execute_approve::ApproveInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        let report = status(&ctx, status_input("plans/checkout/plan.md")).unwrap();

        let board = report.value.board.as_ref().expect("a board exists");
        assert_eq!(board.status, ExecutionStatus::Approved);
        assert_eq!(
            gate(&report.value, Gate::ExecutionGraph).state,
            GateState::Approved
        );
        assert!(!report.value.divergent);
    }

    /// The divergence the predecessor's TS permitted — board `approved` while
    /// the gate is not — is made visible, with the way out named.
    #[test]
    fn status_flags_a_board_gate_divergence() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        let graph = root.join("graph.json");
        fs::write_text(&graph, GRAPH_JSON).unwrap();
        execute_prepare::prepare(
            &ctx,
            execute_prepare::PrepareInput {
                feature: "checkout".to_owned(),
                graph_json: graph.to_string(),
            },
        )
        .unwrap();
        awaiting_approval(&root);
        execute_approve::approve(
            &ctx,
            execute_approve::ApproveInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        // Rewrite the gate to pending behind ivar's back — the TS bug state.
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        let mut approvals = ApprovalState::read(&layout, &feature).unwrap().unwrap();
        approvals.set(Gate::ExecutionGraph, GateState::Pending, None);
        approvals.write(&layout, &feature).unwrap();

        let report = status(&ctx, status_input("plans/checkout/plan.md")).unwrap();

        assert!(report.value.divergent, "the divergence must be reported");
        let mut out = Vec::new();
        report.value.write_human(&mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("DIVERGENCE"),
            "the human surface must make it unmissable: {rendered}"
        );
        assert!(rendered.contains("ivar feature execute approve"));
    }

    #[test]
    fn status_accepts_the_plan_directory_as_the_plan_path() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root);

        let report = status(&ctx, status_input("plans/checkout")).unwrap();

        assert_eq!(report.value.feature.as_str(), "checkout");
    }

    #[test]
    fn status_is_blocked_for_a_path_that_is_not_a_plan() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root);

        let failure = status(&ctx, status_input("README.md")).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "plan.status_not_a_plan");
    }

    #[test]
    fn the_human_surface_lists_gates_and_their_invalidation() {
        let outcome = StatusOutcome {
            root: Utf8PathBuf::from("/hall"),
            feature: FeatureName::new("checkout").unwrap(),
            plan_path: Utf8PathBuf::from("/hall/plans/checkout/plan.md"),
            gates: vec![
                GateStatus {
                    gate: Gate::Requirements,
                    state: GateState::NeedsRevision,
                    invalidated_by: Some(
                        "`plans/checkout/requirements.md` changed since approval".to_owned(),
                    ),
                },
                GateStatus {
                    gate: Gate::Analysis,
                    state: GateState::NeedsRevision,
                    invalidated_by: Some("cascaded from `requirements`".to_owned()),
                },
                GateStatus {
                    gate: Gate::Plan,
                    state: GateState::Approved,
                    invalidated_by: None,
                },
                GateStatus {
                    gate: Gate::ExecutionGraph,
                    state: GateState::Approved,
                    invalidated_by: None,
                },
            ],
            board: Some(BoardStatus {
                board_path: Utf8PathBuf::from("/hall/.ivar/features/checkout/execution/board.json"),
                status: ExecutionStatus::Approved,
                workstreams: 1,
                plan_fingerprint: "abc".to_owned(),
                plan_matches: true,
            }),
            divergent: false,
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "SPDD status for feature `checkout` (plan: /hall/plans/checkout/plan.md):\n\
             \x20 requirements     needs-revision   — `plans/checkout/requirements.md` changed since approval\n\
             \x20 analysis         needs-revision   — cascaded from `requirements`\n\
             \x20 plan             approved\n\
             \x20 execution-graph  approved\n\
             Board: approved (1 workstream) — /hall/.ivar/features/checkout/execution/board.json\n"
        );
    }
}
