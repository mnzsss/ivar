//! `ivar feature execute ack-revision <feature> --workstream <id>` —
//! acknowledge a plan revision for one paused workstream.
//!
//! # What it does
//!
//! A workstream that [`replan`](crate::action::execute::replan) paused is
//! gated: it must not resume until a human acknowledges the revised plan it
//! was paused for. `ack_revision` is that acknowledgment — it unpauses the
//! workstream (back to `Waiting`), records a `replan-acked` journal entry, and
//! when the last paused workstream has acknowledged, resumes the whole board
//! by advancing its status to `Running`.
//!
//! Acknowledging a workstream that is not paused is refused: there is nothing
//! to acknowledge. The gate is the `Paused` status itself, so "every affected
//! workstream has acknowledged" is simply "no workstream is paused".

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{ExecutionBoard, ExecutionStatus, JournalEntry, WorkstreamStatus};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};

use super::super::discover_hall;
use super::{require_board, workstream_not_found};
use crate::action::Ctx;
use crate::store::feature;

/// What `ivar feature execute ack-revision` needs.
#[derive(Debug, Clone)]
pub struct AckInput {
    /// The feature whose board holds the paused workstream.
    pub feature: String,
    /// The paused workstream's id.
    pub workstream: String,
}

/// What `ivar feature execute ack-revision` did.
#[derive(Debug, Clone, Serialize)]
pub struct AckOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the board belongs to.
    pub feature: FeatureName,
    /// The workstream that was unpaused.
    pub workstream: String,
    /// `true` when this was the last paused workstream — the board resumed.
    pub resumed: bool,
    /// The board after the acknowledgment.
    pub board: ExecutionBoard,
    /// Where the board was written.
    pub board_path: Utf8PathBuf,
}

impl WriteHuman for AckOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let resumed = if self.resumed {
            " — execution resumed"
        } else {
            ""
        };
        writeln!(
            w,
            "Acknowledged plan revision for `{}` workstream `{}`{resumed} at {}",
            self.feature, self.workstream, self.board_path
        )
    }
}

/// Acknowledge the plan revision for `input.workstream`, unpausing it.
///
/// Blocked when the feature has no board, the workstream is unknown, or the
/// workstream is not paused — nothing to acknowledge. Unpausing is persisted
/// with its journal entry before the outcome is returned.
pub fn ack_revision(ctx: &Ctx, input: AckInput) -> Outcome<AckOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;

    let mut board = require_board(&layout, &feature)?;
    let board_path = feature::board_path(&layout, &feature);
    let workstream = board
        .graph
        .workstreams
        .iter_mut()
        .find(|workstream| workstream.id == input.workstream)
        .ok_or_else(|| workstream_not_found(&feature, &input.workstream))?;

    if workstream.status != WorkstreamStatus::Paused {
        return Err(Failure::blocked(
            "execute.workstream_not_paused",
            format!(
                "workstream `{}` is not paused — nothing to acknowledge",
                input.workstream
            ),
        )
        .expected("a workstream paused by a plan revision")
        .actual(format!("`{}` is {}", input.workstream, workstream.status))
        .fix(FixAction::safe(
            "execute.ack_paused_only",
            "Acknowledge a workstream only after `feature execute replan` paused it.",
        )));
    }

    workstream.status = WorkstreamStatus::Waiting;
    board.push_journal(JournalEntry::new(
        &input.workstream,
        "replan-acked",
        format!(
            "Acknowledged the plan revision; workstream `{}` unpaused",
            input.workstream
        ),
    ));

    let resumed = board
        .graph
        .workstreams
        .iter()
        .all(|workstream| workstream.status != WorkstreamStatus::Paused);
    if resumed {
        board.set_status(ExecutionStatus::Running);
    }
    board.write(&layout, &feature)?;

    Ok(Report::new(AckOutcome {
        root: layout.root().to_path_buf(),
        feature,
        workstream: input.workstream,
        resumed,
        board,
        board_path,
    }))
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
    use crate::action::execute::replan::{self, ReplanInput};
    use crate::error::Status;
    use crate::infra::fs;
    use crate::store::layout::Layout;
    use crate::test_support::hall_root;

    use crate::action::execute::prepare::{self as prepare_action, PrepareInput};
    use crate::action::feature::create::{
        self as feature_create, CreateInput as FeatureCreateInput,
    };
    use crate::action::hall::{self, InitInput};
    use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};

    const GRAPH_JSON: &str = r#"{
        "workstreams": [
            {
                "id": "ws-a",
                "title": "A",
                "operations": ["op-a1", "op-a2"],
                "depends_on": [],
                "write_contract": ["src/a"]
            },
            {
                "id": "ws-b",
                "title": "B",
                "operations": ["op-b1"],
                "depends_on": ["ws-a"],
                "write_contract": ["src/b"]
            }
        ]
    }"#;

    /// A revision that changes both workstreams' Operations.
    const REVISED_PLAN: &str = "# Plan\n\
        \n\
        ## Operations\n\
        \n\
        ### ws-a\n\
        - op-a1\n\
        - op-a2\n\
        - op-a3\n\
        write_contract:\n\
        - src/a\n\
        \n\
        ### ws-b\n\
        - op-b1\n\
        - op-b2\n\
        write_contract:\n\
        - src/b\n";

    /// A hall with a feature, a plan, and a prepared board of two workstreams,
    /// both paused by a replan.
    fn paused_board() -> (tempfile::TempDir, Utf8PathBuf) {
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
        let plan = root.join("plan-revised.md");
        fs::write_text(&plan, REVISED_PLAN).unwrap();
        replan::replan(
            &ctx,
            ReplanInput {
                feature: "checkout".to_owned(),
                plan: plan.to_string(),
            },
        )
        .unwrap();
        (guard, root)
    }

    fn persisted(root: &Utf8PathBuf) -> ExecutionBoard {
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        ExecutionBoard::read(&layout, &feature).unwrap().unwrap()
    }

    #[test]
    fn ack_unpauses_the_workstream_and_resumes_when_the_last_acknowledges() {
        let (_guard, root) = paused_board();
        let ctx = Ctx::new(root.clone());

        // The first acknowledgment unpauses ws-a but ws-b is still paused.
        let report = ack_revision(
            &ctx,
            AckInput {
                feature: "checkout".to_owned(),
                workstream: "ws-a".to_owned(),
            },
        )
        .unwrap();

        assert!(!report.value.resumed);
        let on_disk = persisted(&root);
        assert_eq!(
            on_disk.graph.workstreams[0].status,
            WorkstreamStatus::Waiting
        );
        assert_eq!(
            on_disk.graph.workstreams[1].status,
            WorkstreamStatus::Paused
        );
        assert_eq!(on_disk.journal.last().unwrap().kind, "replan-acked");

        // The last acknowledgment unpauses ws-b and resumes the board.
        let report = ack_revision(
            &ctx,
            AckInput {
                feature: "checkout".to_owned(),
                workstream: "ws-b".to_owned(),
            },
        )
        .unwrap();

        assert!(report.value.resumed);
        let on_disk = persisted(&root);
        assert_eq!(on_disk.status, ExecutionStatus::Running);
        assert!(
            on_disk
                .graph
                .workstreams
                .iter()
                .all(|workstream| workstream.status == WorkstreamStatus::Waiting)
        );
    }

    #[test]
    fn ack_is_blocked_for_a_workstream_that_is_not_paused() {
        let (_guard, root) = paused_board();
        let ctx = Ctx::new(root.clone());
        // ws-a is paused; acknowledging it first makes ws-b the only paused
        // workstream, then a second ack of ws-a has nothing to do.
        ack_revision(
            &ctx,
            AckInput {
                feature: "checkout".to_owned(),
                workstream: "ws-b".to_owned(),
            },
        )
        .unwrap();

        let failure = ack_revision(
            &ctx,
            AckInput {
                feature: "checkout".to_owned(),
                workstream: "ws-b".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "execute.workstream_not_paused");
    }

    #[test]
    fn ack_is_blocked_for_an_unknown_workstream() {
        let (_guard, root) = paused_board();
        let ctx = Ctx::new(root.clone());

        let failure = ack_revision(
            &ctx,
            AckInput {
                feature: "checkout".to_owned(),
                workstream: "ws-ghost".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "execute.workstream_not_found");
    }
}
