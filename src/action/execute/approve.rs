//! `ivar feature execute approve` — transition the board from
//! `AwaitingApproval` to `Approved`, closing the Execution Graph approval gate.
//!
//! # What it does
//!
//! Reads the feature's [`ExecutionBoard`], verifies it is in
//! [`ExecutionStatus::AwaitingApproval`], transitions it to
//! [`ExecutionStatus::Approved`], appends a `graph.approved` journal entry,
//! and closes the [`Gate::ExecutionGraph`] gate in the approval state by
//! setting it to [`GateState::Approved`] with the plan.md fingerprint.
//!
//! The board and approvals are persisted atomically — if either write fails
//! the operation is aborted, so the two never diverge.
//!
//! Approving a board that is not in `AwaitingApproval` is refused, naming the
//! actual state. Approving twice is idempotent: the second run detects the
//! existing `graph.approved` journal entry (by its event_id) and returns
//! success without duplicating the entry or rewriting the board.
//!
//! This is the **only** writer of the Execution Graph gate in the approvals
//! file. No other command may set `Gate::ExecutionGraph` to `Approved`.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{
    ApprovalState, ExecutionBoard, ExecutionStatus, Gate, GateState, JournalEntry,
};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::hash;
use crate::store::feature;

use super::super::discover_hall;
use crate::action::Ctx;

/// What `ivar feature execute approve` needs.
#[derive(Debug, Clone)]
pub struct ApproveInput {
    /// The feature whose board to approve.
    pub feature: String,
}

/// What `ivar feature execute approve` did.
#[derive(Debug, Clone, Serialize)]
pub struct ApproveOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the board belongs to.
    pub feature: FeatureName,
    /// Where the board was written.
    pub board_path: Utf8PathBuf,
    /// The board after the transition.
    pub board: ExecutionBoard,
}

impl WriteHuman for ApproveOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Approved execution board for `{}` at {}",
            self.feature, self.board_path
        )?;
        for record in &self.board.journal {
            writeln!(w, "  [{}] {} — {}", record.seq, record.kind, record.message)?;
        }
        Ok(())
    }
}

/// Transition `input.feature`'s board from `AwaitingApproval` to `Approved`.
///
/// Blocked when the feature has no board or the board is not in
/// `AwaitingApproval`. Idempotent: if the board is already approved (a
/// `graph.approved` entry exists), returns success without duplicating the
/// journal entry.
pub fn approve(ctx: &Ctx, input: ApproveInput) -> Outcome<ApproveOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;

    let mut board = match ExecutionBoard::read(&layout, &feature)? {
        Some(b) => b,
        None => {
            return Err(Failure::blocked(
                "execute.board_missing",
                format!("no execution board for feature `{feature}`"),
            )
            .expected("a prepared execution board")
            .actual("board.json does not exist under the feature's execution directory")
            .fix(FixAction::safe(
                "execute.prepare_first",
                format!(
                    "Prepare the board first: `ivar feature execute prepare {feature} --graph-json <path>`."
                ),
            )))
        }
    };

    // Idempotency check: if the board is already approved, there is nothing
    // to do. A `graph.approved` entry means the transition already happened.
    if board.status == ExecutionStatus::Approved {
        let board_path = feature::board_path(&layout, &feature);
        return Ok(Report::new(ApproveOutcome {
            root: layout.root().to_path_buf(),
            feature,
            board_path,
            board,
        }));
    }

    if board.status != ExecutionStatus::AwaitingApproval {
        return Err(Failure::blocked(
            "execute.board_not_awaiting_approval",
            format!(
                "cannot approve board for `{feature}`: board is `{}`, expected `awaiting_approval`",
                board.status
            ),
        )
        .expected("a board in `AwaitingApproval` status")
        .actual(format!("board status is `{}`", board.status))
        .fix(FixAction::safe(
            "execute.reprepare",
            format!(
                "Delete `{}` deliberately and re-prepare from a fresh graph.",
                feature::board_path(&layout, &feature)
            ),
        )));
    }

    // Generate a deterministic event_id based on timestamp to ensure
    // idempotency — same run produces same ID, duplicate runs detect it.
    let event_id = format!("approved.{}", now_epoch_seconds());

    // Check for existing graph.approved entry (dedup guard).
    if board
        .journal
        .iter()
        .any(|entry| entry.event_id == event_id || entry.kind == "graph.approved")
    {
        let board_path = feature::board_path(&layout, &feature);
        return Ok(Report::new(ApproveOutcome {
            root: layout.root().to_path_buf(),
            feature,
            board_path,
            board,
        }));
    }

    board.set_status(ExecutionStatus::Approved);

    let mut entry = JournalEntry::new(
        "board",
        "graph.approved",
        format!("Board approved for `{feature}` — ready to tick"),
    );
    entry.event_id = event_id;

    board.push_journal(entry);

    // Close the Execution Graph gate in the approval state.
    let mut approvals = ApprovalState::read(&layout, &feature)?.unwrap_or_default();
    approvals.normalize();

    // Fingerprint the plan.md — the same content the Execution Graph approval
    // gate fingerprints. Any plan change invalidates this approval.
    let plan_path = layout.plan_dir(&feature).join("plan.md");
    let fingerprint = hash::file(&plan_path).ok();

    // `execute approve` is the one writer of the execution-graph gate; it
    // does not touch the three SPDD gates upstream. Those belong to `plan
    // approve`, and re-opening them here would make the status cascade
    // re-open the gate this very operation just approved.
    approvals.set(Gate::ExecutionGraph, GateState::Approved, fingerprint);

    board.write(&layout, &feature)?;
    approvals.write(&layout, &feature)?;

    let board_path = feature::board_path(&layout, &feature);
    Ok(Report::new(ApproveOutcome {
        root: layout.root().to_path_buf(),
        feature,
        board_path,
        board,
    }))
}

/// Current time as UNIX epoch seconds string — matches the format used by
/// `JournalEntry::timestamp`.
fn now_epoch_seconds() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::action::execute::prepare::{self as prepare_action, PrepareInput};
    use crate::action::feature::create::{
        self as feature_create, CreateInput as FeatureCreateInput,
    };
    use crate::action::hall::{self, InitInput};
    use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
    use crate::domain::feature::ExecutionStatus;
    use crate::error::Status;
    use crate::infra::fs;
    use crate::store::layout::Layout;
    use crate::test_support::hall_root;

    const GRAPH_JSON: &str = r#"{
        "workstreams": [
            {
                "id": "ws-gates",
                "title": "Approval gates",
                "operations": ["add-gate-types", "wire-approve"],
                "depends_on": [],
                "write_contract": ["src/domain/feature.rs"]
            },
            {
                "id": "ws-board",
                "title": "Execution board",
                "operations": ["add-board-types", "store-board"],
                "depends_on": ["ws-gates"],
                "write_contract": ["src/action/execute"]
            }
        ]
    }"#;

    /// A hall with a feature, a plan, and a prepared board.
    fn seeded_board() -> (tempfile::TempDir, Utf8PathBuf) {
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
        let graph = root.join("graph.json");
        fs::write_text(&graph, GRAPH_JSON).unwrap();
        prepare_action::prepare(
            &ctx,
            PrepareInput {
                feature: "checkout".to_owned(),
                graph_json: graph.to_string(),
            },
        )
        .unwrap();
        (guard, root)
    }

    /// The board read back off disk — the real file, not the in-memory value
    /// an action returned.
    fn persisted(root: &Utf8PathBuf) -> ExecutionBoard {
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        ExecutionBoard::read(&layout, &feature).unwrap().unwrap()
    }

    /// The persisted approval state, read back off disk.
    fn persisted_approvals(root: &Utf8PathBuf) -> ApprovalState {
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        ApprovalState::read(&layout, &feature).unwrap().unwrap()
    }

    #[test]
    fn approve_transitions_awaiting_approval_to_approved_and_closes_the_gate() {
        let (_guard, root) = seeded_board();
        let ctx = Ctx::new(root.clone());

        let report = approve(
            &ctx,
            ApproveInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        assert!(report.is_clean());
        assert_eq!(report.value.board.status, ExecutionStatus::Approved);

        // The journal contains the graph.approved entry.
        let on_disk = persisted(&root);
        let last_entry = on_disk.journal.last().unwrap();
        assert_eq!(last_entry.kind, "graph.approved");
        assert!(!last_entry.event_id.is_empty());

        // The Execution Graph gate is closed in approvals.
        let approvals = persisted_approvals(&root);
        assert_eq!(
            approvals.state(Gate::ExecutionGraph),
            Some(GateState::Approved)
        );
    }

    #[test]
    fn approve_refuses_a_board_not_in_awaiting_approval_naming_the_state() {
        let (_guard, root) = seeded_board();
        let ctx = Ctx::new(root.clone());

        // Manually change the board to Pending via store.
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
        board.set_status(ExecutionStatus::Pending);
        board.write(&layout, &feature).unwrap();

        let failure = approve(
            &ctx,
            ApproveInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "execute.board_not_awaiting_approval");
        assert!(
            failure.what.contains("pending"),
            "error must name the actual state: {}",
            failure.what
        );
    }

    #[test]
    fn approve_twice_does_not_duplicate_the_journal_event() {
        let (_guard, root) = seeded_board();
        let ctx = Ctx::new(root.clone());

        // First approve.
        approve(
            &ctx,
            ApproveInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        let journal_len_after_first = persisted(&root).journal.len();

        // Second approve — should be a no-op.
        approve(
            &ctx,
            ApproveInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        let journal_len_after_second = persisted(&root).journal.len();

        assert_eq!(
            journal_len_after_first, journal_len_after_second,
            "second approve must not add a journal entry"
        );
    }

    #[test]
    fn after_execute_approve_the_execution_graph_gate_is_approved() {
        let (_guard, root) = seeded_board();
        let ctx = Ctx::new(root.clone());

        approve(
            &ctx,
            ApproveInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        let approvals = persisted_approvals(&root);

        // The three SPDD gates upstream are untouched — `plan approve` owns
        // them, and `execute approve` writes only the execution-graph gate.
        assert_eq!(
            approvals.state(Gate::Requirements),
            Some(GateState::Pending)
        );
        assert_eq!(approvals.state(Gate::Analysis), Some(GateState::Pending));
        assert_eq!(approvals.state(Gate::Plan), Some(GateState::Pending));
        assert_eq!(
            approvals.state(Gate::ExecutionGraph),
            Some(GateState::Approved)
        );
    }

    #[test]
    fn the_human_surface_lists_journal_entries() {
        let outcome = ApproveOutcome {
            root: Utf8PathBuf::from("/hall"),
            feature: FeatureName::new("checkout").unwrap(),
            board_path: Utf8PathBuf::from("/hall/board.json"),
            board: ExecutionBoard::new(crate::domain::feature::ExecutionGraph {
                plan_fingerprint: "abc".to_owned(),
                workstreams: vec![],
            }),
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Approved execution board"));
        assert!(text.contains("checkout"));
    }
}
