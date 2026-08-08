//! `ivar feature execute tick` — find ready workstreams and launch them.
//!
//! # What it does
//!
//! Reads the feature's [`ExecutionBoard`], finds workstreams whose declared
//! [`WorkstreamDef::depends_on`] are all [`WorkstreamStatus::Done`], and for
//! each:
//!
//! 1. Validates the plan fingerprint against the board; if diverged, marks the
//!    workstream [`WorkstreamStatus::Blocked`] and does NOT launch.
//! 2. Launches a provider session (using the hall's manifest default or the
//!    workstream's own `provider`/`agent`), recording the session id in
//!    [`ExecutionBoard::sessions`].
//! 3. Transitions the workstream from [`WorkstreamStatus::Waiting`] to
//!    [`WorkstreamStatus::Active`].
//!
//! The board's overall status advances from [`ExecutionStatus::Approved`] to
//! [`ExecutionStatus::Running`] once at least one workstream is launched.
//!
//! Tick with nothing ready is a no-op that reports so. Never launches real
//! providers — uses a fake harness adapter that returns a command string.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{ExecutionBoard, ExecutionStatus, JournalEntry, WorkstreamStatus};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::hash;
use crate::store::feature;

use super::super::discover_hall;
use crate::action::Ctx;

/// What `ivar feature execute tick` needs.
#[derive(Debug, Clone)]
pub struct TickInput {
    /// The feature whose board to tick — find ready workstreams and launch them.
    pub feature: String,
}

/// What `ivar feature execute tick` did.
#[derive(Debug, Clone, Serialize)]
pub struct TickOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the board belongs to.
    pub feature: FeatureName,
    /// Workstreams that were launched, in order.
    pub launched: Vec<String>,
    /// The board after the tick.
    pub board: ExecutionBoard,
    /// Where the board was written.
    pub board_path: Utf8PathBuf,
}

impl WriteHuman for TickOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.launched.is_empty() {
            writeln!(
                w,
                "Tick: nothing ready for `{}` — no workstreams to launch",
                self.feature
            )?;
        } else {
            writeln!(
                w,
                "Tick: launched {} for `{}` at {}",
                if self.launched.len() == 1 {
                    "workstream"
                } else {
                    "workstreams"
                },
                self.feature,
                self.board_path
            )?;
            for ws in &self.launched {
                writeln!(w, "  - {ws}")?;
            }
        }
        Ok(())
    }
}

/// Find ready workstreams on `input.feature`'s board and launch them.
///
/// Blocked when the feature has no board or the board is not in
/// `Approved` status — only an approved board may be ticked. A divergent
/// plan fingerprint blocks individual workstreams rather than launching them.
/// Tick with nothing ready is a no-op.
pub fn tick(ctx: &Ctx, input: TickInput) -> Outcome<TickOutcome> {
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

    if board.status != ExecutionStatus::Approved {
        return Err(Failure::blocked(
            "execute.board_not_approved",
            format!(
                "cannot tick board for `{feature}`: board is `{}`, expected `approved`",
                board.status
            ),
        )
        .expected("an approved board")
        .actual(format!("board status is `{}`", board.status))
        .fix(FixAction::safe(
            "execute.approve_first",
            format!("Approve the board first: `ivar feature execute approve {feature}`."),
        )));
    }

    // Validate plan fingerprint against the board. If diverged, block ALL
    // waiting workstreams and do NOT launch any.
    let board_fingerprint = board.graph.plan_fingerprint.clone();
    let plan_path = layout.plan_dir(&feature).join("plan.md");
    let current_fingerprint = hash::file(&plan_path).ok();

    if let Some(fp) = &current_fingerprint {
        if fp != &board_fingerprint {
            // Divergent plan — block all waiting workstreams.
            for ws in &mut board.graph.workstreams {
                if ws.status == WorkstreamStatus::Waiting {
                    ws.status = WorkstreamStatus::Blocked;
                }
            }
            board.push_journal(JournalEntry::new(
                "board",
                "diverged",
                format!(
                    "Plan diverged from board fingerprint (expected {}, got {})",
                    board_fingerprint, fp
                ),
            ));
            board.write(&layout, &feature)?;

            let board_path = feature::board_path(&layout, &feature);
            return Ok(Report::new(TickOutcome {
                root: layout.root().to_path_buf(),
                feature,
                launched: Vec::new(),
                board,
                board_path,
            }));
        }
    }

    let mut launched = Vec::new();
    let mut to_launch = Vec::new();

    // First pass: identify which workstreams can be launched.
    for ws in &board.graph.workstreams {
        if ws.status != WorkstreamStatus::Waiting {
            continue;
        }
        // Check dependencies: all must be Done.
        let deps_met = ws.depends_on.iter().all(|dep_id| {
            board
                .graph
                .workstreams
                .iter()
                .any(|w| w.id == *dep_id && w.status == WorkstreamStatus::Done)
        });
        if !deps_met {
            continue;
        }
        to_launch.push(ws.id.clone());
    }

    // Second pass: launch the identified workstreams. Collect session data first,
    // then apply mutations after releasing the workstream borrow.
    let mut sessions_to_add = Vec::new();
    let mut journals_to_add = Vec::new();
    let mut status_updates = Vec::new();

    for workstream in &mut board.graph.workstreams {
        if !to_launch.contains(&workstream.id) {
            continue;
        }

        // Build the provider command. The agent reaches the provider's
        // command line as a --model argument. This is the bifrost bug fix:
        // the old adapter built ['run', '-p', prompt] and discarded the
        // agent/model entirely.
        let binary = workstream
            .provider
            .map_or_else(|| "claude-code".to_owned(), |p| p.to_string());
        let cmd_str = if let Some(agent) = &workstream.agent {
            format!("{binary} --model {agent}")
        } else {
            binary.to_string()
        };

        // Generate a session ID.
        let session_id = uuid::Uuid::new_v4().to_string();

        workstream.status = WorkstreamStatus::Active;
        status_updates.push(workstream.id.clone());
        sessions_to_add.push((session_id.clone(), workstream.id.clone()));
        journals_to_add.push(JournalEntry {
            seq: 0, // placeholder — filled below
            event_id: format!("started.{session_id}"),
            timestamp: now_epoch_seconds(),
            workstream: workstream.id.clone(),
            kind: "started".to_owned(),
            message: format!("Launched session {session_id} ({cmd_str})"),
        });

        launched.push(workstream.id.clone());
    }

    // Apply session links and journal entries.
    for (sid, wsid) in sessions_to_add {
        board.sessions.insert(sid, wsid);
    }
    // Assign seq numbers and push journals.
    for entry in &mut journals_to_add {
        board.next_event_seq += 1;
        entry.seq = board.next_event_seq;
        board.push_journal(entry.clone());
    }

    // Advance board status if we launched anything.
    if !launched.is_empty() && board.status == ExecutionStatus::Approved {
        board.set_status(ExecutionStatus::Running);
    }

    board.write(&layout, &feature)?;

    let board_path = feature::board_path(&layout, &feature);
    Ok(Report::new(TickOutcome {
        root: layout.root().to_path_buf(),
        feature,
        launched,
        board,
        board_path,
    }))
}

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
    use crate::action::execute::approve::{self as approve_action, ApproveInput};
    use crate::action::execute::prepare::{self as prepare_action, PrepareInput};
    use crate::action::feature::create::{
        self as feature_create, CreateInput as FeatureCreateInput,
    };
    use crate::action::hall::{self, InitInput};
    use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
    use crate::error::Status;
    use crate::infra::fs;
    use crate::store::layout::Layout;
    use crate::test_support::hall_root;

    const GRAPH_JSON: &str = r#"{
        "workstreams": [
            {
                "id": "ws-a",
                "title": "A",
                "operations": ["op-a"],
                "depends_on": [],
                "write_contract": ["src/a"]
            },
            {
                "id": "ws-b",
                "title": "B",
                "operations": ["op-b"],
                "depends_on": ["ws-a"],
                "write_contract": ["src/b"]
            }
        ]
    }"#;

    /// A hall with a feature, a plan, and a prepared+approved board.
    fn approved_board() -> (tempfile::TempDir, Utf8PathBuf) {
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
        approve_action::approve(
            &ctx,
            ApproveInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();
        (guard, root)
    }

    /// The board read back off disk.
    fn persisted(root: &Utf8PathBuf) -> ExecutionBoard {
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        ExecutionBoard::read(&layout, &feature).unwrap().unwrap()
    }

    #[test]
    fn tick_launches_workstreams_with_met_dependencies() {
        let (_guard, root) = approved_board();
        let ctx = Ctx::new(root.clone());

        // Mark ws-a as Done so ws-b becomes ready.
        {
            let layout = Layout::at(root.clone());
            let feature = FeatureName::new("checkout").unwrap();
            let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
            for ws in &mut board.graph.workstreams {
                if ws.id == "ws-a" {
                    ws.status = WorkstreamStatus::Done;
                }
            }
            board.write(&layout, &feature).unwrap();
        }

        let report = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        assert!(report.is_clean());
        assert_eq!(report.value.launched, vec!["ws-b"]);
        assert_eq!(persisted(&root).status, ExecutionStatus::Running);

        // Session→workstream link recorded.
        let on_disk = persisted(&root);
        assert_eq!(on_disk.sessions.len(), 1);
        let (sess_id, ws_id) = on_disk.sessions.iter().next().unwrap();
        assert_eq!(ws_id, "ws-b");
        assert!(!sess_id.is_empty());
    }

    #[test]
    fn tick_blocks_when_plan_diverges() {
        let (_guard, root) = approved_board();
        let ctx = Ctx::new(root.clone());

        // Tamper with plan.md behind ivar's back — changes the fingerprint.
        fs::write_text(
            &root.join("plans/checkout/plan.md"),
            "# Plan\n\n- changed content\n",
        )
        .unwrap();

        let report = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        assert!(report.is_clean());
        assert!(
            report.value.launched.is_empty(),
            "divergent plan must block"
        );

        // Workstream marked as Blocked.
        let on_disk = persisted(&root);
        assert_eq!(
            on_disk.graph.workstreams[0].status,
            WorkstreamStatus::Blocked
        );

        // Journal records the divergence.
        let last_entry = on_disk.journal.last().unwrap();
        assert_eq!(last_entry.kind, "diverged");
    }

    #[test]
    fn tick_with_nothing_ready_is_a_no_op() {
        let (_guard, root) = approved_board();
        let ctx = Ctx::new(root.clone());

        // No workstream is Done, so ws-b's dependency is unmet.
        // ws-a has no dependencies but... actually ws-a HAS no deps and IS
        // Waiting, so it should launch. Let me check: depends_on is empty
        // for ws-a, so deps_met is true, meaning ws-a WILL launch.
        // To test "nothing ready", I need to set up a scenario where no
        // workstream can launch. Let me modify the setup.

        // Actually, with the current graph, ws-a has no deps and will always
        // launch. Let me create a board where ws-a depends on itself (circular)
        // or just accept that ws-a launches and test differently.

        // For "nothing ready" test, let me manually set up a board where
        // ws-a depends on ws-b which depends on ws-a (circular deps).
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();

        // Make both depend on each other — circular, neither can launch.
        board.graph.workstreams[0].depends_on = vec!["ws-b".to_owned()];
        board.graph.workstreams[1].depends_on = vec!["ws-a".to_owned()];

        board.write(&layout, &feature).unwrap();

        let report = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        assert!(report.is_clean());
        assert!(
            report.value.launched.is_empty(),
            "circular deps: nothing should launch"
        );
    }

    #[test]
    fn tick_refuses_a_board_not_in_approved_status() {
        let (_guard, root) = approved_board();
        let ctx = Ctx::new(root.clone());

        // Set board to Pending.
        {
            let layout = Layout::at(root.clone());
            let feature = FeatureName::new("checkout").unwrap();
            let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
            board.set_status(ExecutionStatus::Pending);
            board.write(&layout, &feature).unwrap();
        }

        let failure = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "execute.board_not_approved");
        assert!(failure.what.contains("pending"));
    }

    #[test]
    fn the_agent_reaches_the_provider_command_line() {
        let (_guard, root) = approved_board();
        let ctx = Ctx::new(root.clone());

        // Give ws-a a custom agent.
        {
            let layout = Layout::at(root.clone());
            let feature = FeatureName::new("checkout").unwrap();
            let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
            if let Some(ws) = board
                .graph
                .workstreams
                .iter_mut()
                .find(|ws| ws.id == "ws-a")
            {
                ws.agent = Some("custom-agent".to_owned());
            }
            board.write(&layout, &feature).unwrap();
        }

        let _report = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        // Verify via journal entry that the agent appears in the command.
        let on_disk = persisted(&root);
        let started_entries: Vec<_> = on_disk
            .journal
            .iter()
            .filter(|e| e.kind == "started")
            .collect();
        assert!(!started_entries.is_empty(), "must have started entries");
        let msg = &started_entries[0].message;
        assert!(msg.contains("custom-agent"), "agent must reach CLI: {msg}");
    }

    #[test]
    fn the_session_to_workstream_link_is_recorded() {
        let (_guard, root) = approved_board();
        let ctx = Ctx::new(root.clone());

        let _report = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        let on_disk = persisted(&root);
        assert_eq!(
            on_disk.sessions.len(),
            1,
            "exactly one session launched (ws-a)"
        );
        let (session_id, workstream_id) = on_disk.sessions.iter().next().unwrap();
        assert_eq!(workstream_id, "ws-a");
        // Session ID is a valid UUID.
        assert!(uuid::Uuid::parse_str(session_id).is_ok());
    }
}
