//! `ivar feature execute replan <feature> --plan <path>` — fold a revised
//! plan into an existing execution board.
//!
//! # What it does
//!
//! Reads the feature's [`ExecutionBoard`], fingerprints the revised plan the
//! caller points at (a new `plan.md`), and compares the two. When the
//! fingerprint is unchanged there is nothing to do and the board is left
//! untouched. When it changed, the board's `plan_fingerprint` advances to the
//! new one, every workstream whose **Operations** changed is **paused** until
//! a human acknowledges the new revision ([`crate::action::execute::ack`]),
//! and the journal records the replan with the new fingerprint.
//!
//! Unaffected workstreams are untouched — their status is left exactly as it
//! was, so they continue. A workstream paused by an *earlier* replan revision
//! stays paused until it is acknowledged; pausing is the gate, and only
//! `ack-revision` lifts it.
//!
//! Replanning never rewrites the board's workstream definitions. The graph
//! keeps the operations it was prepared with, because that is the "old plan"
//! every later replan diffs against; executors read the current Operations
//! from `plan.md` itself.
//!
//! # The plan's Operations section
//!
//! The revised `plan.md` carries the new Operations in a section this verb
//! parses: a heading whose text is `Operations`, then one subheading per
//! workstream named by its id, with `- ` bullets as its operations. A
//! `write_contract:` line switches the bullets that follow to the write
//! contract. Example:
//!
//! ```text
//! ## Operations
//!
//! ### ws-board
//! - add-board-types
//! - store-board
//! write_contract:
//! - src/action/execute
//! ```
//!
//! A workstream whose subheading is absent from the revised plan counts as
//! affected — its Operations are gone, so its executor must review the change.
//! The section runs to the end of the file; every heading inside it (besides
//! the `Operations` heading itself) names a workstream.
//!
//! # v1 scope
//!
//! Affected detection is whole-workstream: any difference in `operations` or
//! `write_contract` pauses the whole workstream. No per-operation inbox
//! granularity yet.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::feature::{ExecutionBoard, JournalEntry, WorkstreamDef, WorkstreamStatus};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::{fs, hash};

use super::super::discover_hall;
use super::require_board;
use crate::action::Ctx;
use crate::store::feature;

/// What `ivar feature execute replan` needs.
#[derive(Debug, Clone)]
pub struct ReplanInput {
    /// The feature whose board is replanned.
    pub feature: String,
    /// Path to the revised `plan.md`. Resolved against the current directory.
    pub plan: String,
}

/// What `ivar feature execute replan` did.
#[derive(Debug, Clone, Serialize)]
pub struct ReplanOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the board belongs to.
    pub feature: FeatureName,
    /// SHA-256 of the revised plan — the board's new `plan_fingerprint`.
    pub fingerprint: String,
    /// `false` when the plan was unchanged and nothing was written.
    pub changed: bool,
    /// The workstreams this replan paused, in board order.
    pub affected: Vec<String>,
    /// The board after the replan.
    pub board: ExecutionBoard,
    /// Where the board was written.
    pub board_path: Utf8PathBuf,
}

impl WriteHuman for ReplanOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.changed {
            let noun = if self.affected.len() == 1 {
                "workstream"
            } else {
                "workstreams"
            };
            writeln!(
                w,
                "Replanned `{}` to {} ({} affected {noun}) at {}",
                self.feature,
                self.fingerprint,
                self.affected.len(),
                self.board_path
            )
        } else {
            writeln!(
                w,
                "Plan for `{}` unchanged ({}); nothing to replan",
                self.feature, self.fingerprint
            )
        }
    }
}

/// Fold the revised plan at `input.plan` into `input.feature`'s board.
///
/// Blocked when the feature has no board yet — replanning advances an
/// existing board; it does not create one. A plan whose fingerprint matches
/// the board's is a no-op: no journal entry, no write. Otherwise the
/// fingerprint advances, affected workstreams pause, and the replan is
/// journaled before the board is persisted.
pub fn replan(ctx: &Ctx, input: ReplanInput) -> Outcome<ReplanOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;
    let plan_path = ctx.resolve(Utf8Path::new(&input.plan));

    let mut board = require_board(&layout, &feature)?;
    let plan_text = read_plan(&plan_path)?;
    let fingerprint = hash::text(&plan_text);
    let board_path = feature::board_path(&layout, &feature);

    if fingerprint == board.graph.plan_fingerprint {
        return Ok(Report::new(ReplanOutcome {
            root: layout.root().to_path_buf(),
            feature,
            fingerprint,
            changed: false,
            affected: Vec::new(),
            board,
            board_path,
        }));
    }

    let revised = operations_from_plan(&plan_text);
    let affected: Vec<String> = board
        .graph
        .workstreams
        .iter()
        .filter(|workstream| is_affected(workstream, &revised))
        .map(|workstream| workstream.id.clone())
        .collect();

    for workstream in &mut board.graph.workstreams {
        if affected.contains(&workstream.id) {
            workstream.status = WorkstreamStatus::Paused;
        }
    }

    board.graph.plan_fingerprint = fingerprint.clone();
    board.push_journal(JournalEntry::new(
        "board",
        "replan",
        format!(
            "Plan revised to fingerprint {fingerprint}; affected workstreams: {}",
            if affected.is_empty() {
                "none".to_owned()
            } else {
                affected.join(", ")
            }
        ),
    ));
    board.write(&layout, &feature)?;

    Ok(Report::new(ReplanOutcome {
        root: layout.root().to_path_buf(),
        feature,
        fingerprint,
        changed: true,
        affected,
        board,
        board_path,
    }))
}

/// Read the revised plan at `path`. Blocked when the file does not exist — a
/// replan against a path that has nothing to read is a mistake, not an empty
/// revision.
fn read_plan(path: &Utf8Path) -> Result<String, Failure> {
    fs::read_text(path)?.ok_or_else(|| {
        Failure::blocked("execute.plan_missing", format!("`{}` does not exist", path))
            .expected("the revised plan.md at the given path")
            .actual("no such file")
            .fix(FixAction::safe(
                "execute.provide_plan",
                "Point --plan at the revised plan.md.",
            ))
    })
}

/// Whether `workstream`'s Operations changed between the board (the old plan)
/// and the revised plan: its `operations` or `write_contract` differ, or its
/// subheading is absent from the revised Operations section entirely.
fn is_affected(workstream: &WorkstreamDef, revised: &[PlanWorkstream]) -> bool {
    match revised.iter().find(|entry| entry.id == workstream.id) {
        Some(entry) => {
            entry.operations != workstream.operations
                || entry.write_contract != workstream.write_contract
        }
        None => true,
    }
}

/// One workstream's Operations as authored in the revised plan.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanWorkstream {
    /// The workstream's id — the subheading text under `Operations`.
    id: String,
    /// The operations, in order.
    operations: Vec<String>,
    /// The paths the workstream may touch.
    write_contract: Vec<String>,
}

/// Parse `text`'s Operations section. See the module doc comment for the
/// exact format; a plan without an Operations section yields an empty list,
/// which makes every board workstream affected — the conservative answer
/// when the new plan carries no operations at all.
fn operations_from_plan(text: &str) -> Vec<PlanWorkstream> {
    let mut workstreams = Vec::new();
    let mut in_operations = false;
    let mut collecting_write_contract = false;
    let mut current: Option<PlanWorkstream> = None;

    for line in text.lines() {
        let trimmed = line.trim();

        if let Some(heading) = trimmed.strip_prefix('#') {
            let title = heading.trim_start_matches('#').trim();
            if title.eq_ignore_ascii_case("operations") {
                // The section (re)starts; whatever workstream was open ends.
                if let Some(workstream) = current.take() {
                    workstreams.push(workstream);
                }
                in_operations = true;
                collecting_write_contract = false;
                continue;
            }
            if !in_operations {
                continue;
            }
            // Any other heading inside the section starts a new workstream,
            // named by the heading text.
            if let Some(workstream) = current.take() {
                workstreams.push(workstream);
            }
            current = Some(PlanWorkstream {
                id: title.to_owned(),
                operations: Vec::new(),
                write_contract: Vec::new(),
            });
            collecting_write_contract = false;
            continue;
        }

        if !in_operations {
            continue;
        }
        let Some(workstream) = current.as_mut() else {
            continue;
        };
        if trimmed == "write_contract:" {
            collecting_write_contract = true;
            continue;
        }
        if let Some(bullet) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let item = bullet.trim().to_owned();
            if collecting_write_contract {
                workstream.write_contract.push(item);
            } else {
                workstream.operations.push(item);
            }
        }
    }
    if let Some(workstream) = current {
        workstreams.push(workstream);
    }

    workstreams
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
    use crate::error::Status;
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

    /// The revised plan: `ws-board` gains an operation, `ws-gates` is
    /// unchanged.
    const REVISED_PLAN: &str = "# Plan\n\
        \n\
        ## Operations\n\
        \n\
        ### ws-gates\n\
        - add-gate-types\n\
        - wire-approve\n\
        write_contract:\n\
        - src/domain/feature.rs\n\
        \n\
        ### ws-board\n\
        - add-board-types\n\
        - store-board\n\
        - tick-board\n\
        write_contract:\n\
        - src/action/execute\n";

    /// A hall with a feature, a plan, and a prepared board (two workstreams).
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

    fn replan_input(root: &Utf8PathBuf, plan: &str) -> ReplanInput {
        let plan_path = root.join("plan-revised.md");
        fs::write_text(&plan_path, plan).unwrap();
        ReplanInput {
            feature: "checkout".to_owned(),
            plan: plan_path.to_string(),
        }
    }

    #[test]
    fn replan_advances_the_fingerprint_pauses_affected_workstreams_and_journals() {
        let (_guard, root) = seeded_board();
        let ctx = Ctx::new(root.clone());
        let input = replan_input(&root, REVISED_PLAN);
        let expected_fingerprint = hash::file(&ctx.resolve(Utf8Path::new(&input.plan))).unwrap();

        let report = replan(&ctx, input).unwrap();

        assert!(report.is_clean());
        assert!(report.value.changed);
        assert_eq!(report.value.fingerprint, expected_fingerprint);
        // Only ws-board's Operations changed in the revised plan.
        assert_eq!(report.value.affected, vec!["ws-board".to_owned()]);

        // The board on disk carries the new fingerprint, the pause, and the
        // replan journal entry.
        let on_disk = persisted(&root);
        assert_eq!(on_disk.graph.plan_fingerprint, expected_fingerprint);
        assert_eq!(
            on_disk.graph.workstreams[0].status,
            WorkstreamStatus::Waiting,
            "unaffected workstreams continue"
        );
        assert_eq!(
            on_disk.graph.workstreams[1].status,
            WorkstreamStatus::Paused,
            "affected workstreams pause until acknowledged"
        );
        let entry = on_disk.journal.last().unwrap();
        assert_eq!(entry.kind, "replan");
        assert_eq!(entry.workstream, "board");
        assert!(entry.message.contains(&expected_fingerprint));
        assert!(entry.message.contains("ws-board"));
    }

    #[test]
    fn replan_is_a_no_op_when_the_plan_is_unchanged() {
        let (_guard, root) = seeded_board();
        let ctx = Ctx::new(root.clone());
        // The scaffolded plan.md the board was prepared against — same bytes.
        let plan = root.join("plans/checkout/plan.md").to_string();
        let input = ReplanInput {
            feature: "checkout".to_owned(),
            plan,
        };
        let journal_len_before = persisted(&root).journal.len();
        let fingerprint_before = persisted(&root).graph.plan_fingerprint;

        let report = replan(&ctx, input).unwrap();

        assert!(!report.value.changed);
        assert_eq!(report.value.fingerprint, fingerprint_before);
        assert!(report.value.affected.is_empty());
        // Nothing was written: the journal did not grow and every workstream
        // is still waiting.
        let on_disk = persisted(&root);
        assert_eq!(on_disk.journal.len(), journal_len_before);
        assert!(
            on_disk
                .graph
                .workstreams
                .iter()
                .all(|workstream| workstream.status == WorkstreamStatus::Waiting)
        );
    }

    #[test]
    fn replan_is_blocked_without_a_board() {
        let (_guard, root) = hall_root();
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
        let input = replan_input(&root, REVISED_PLAN);

        let failure = replan(&ctx, input).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "execute.board_missing");
    }

    #[test]
    fn replan_is_blocked_when_the_plan_path_does_not_exist() {
        let (_guard, root) = seeded_board();
        let ctx = Ctx::new(root.clone());

        let failure = replan(
            &ctx,
            ReplanInput {
                feature: "checkout".to_owned(),
                plan: "does-not-exist.md".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "execute.plan_missing");
    }

    #[test]
    fn operations_from_plan_parses_ids_operations_and_write_contracts() {
        let parsed = operations_from_plan(REVISED_PLAN);

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, "ws-gates");
        assert_eq!(
            parsed[0].operations,
            vec!["add-gate-types".to_owned(), "wire-approve".to_owned()]
        );
        assert_eq!(
            parsed[0].write_contract,
            vec!["src/domain/feature.rs".to_owned()]
        );
        assert_eq!(parsed[1].id, "ws-board");
        assert_eq!(parsed[1].operations.len(), 3);

        // A plan with no Operations section parses to nothing — and every
        // board workstream therefore counts as affected.
        assert!(operations_from_plan("# Plan\n\nprose only\n").is_empty());
    }
}
