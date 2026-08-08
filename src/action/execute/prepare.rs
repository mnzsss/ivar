//! `ivar feature execute <feature> --graph-json <path>` — prepare a
//! feature's execution board.
//!
//! # What it does
//!
//! Reads the feature's plan (`plans/<feature>/plan.md`) and the execution
//! graph the plan derives — a plain JSON file of workstreams, each with
//! `id`, `title`, `operations`, `depends_on` and `write_contract` — and
//! writes an [`ExecutionBoard`] at
//! `features/<feature>/execution/board.json` (schema v1, `Policy::Local`).
//!
//! The graph file carries no execution state: `prepare` stamps the board's
//! status `Pending`, every workstream's status `Waiting`, and fingerprints
//! `plan.md` into the graph so a plan change voids the board (the same
//! content the Execution Graph approval gate fingerprints). The board's
//! journal opens with a `prepared` entry.
//!
//! Preparing is a one-shot: a feature that already has a board is refused,
//! because re-writing it would destroy the journal. Delete `board.json`
//! deliberately to re-prepare from a fresh graph.
//!
//! # v1 scope
//!
//! Graph + status + journal only. No inboxes, no blockers, no handoffs, no
//! tick/reply — nothing here advances the board once it exists.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::domain::feature::{
    ExecutionBoard, ExecutionGraph, JournalEntry, WorkstreamDef, WorkstreamStatus,
};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::{fs, hash, json};
use crate::store::feature;
use crate::store::layout::Layout;

use super::super::discover_hall;
use crate::action::Ctx;

/// What `ivar feature execute` needs.
#[derive(Debug, Clone)]
pub struct PrepareInput {
    /// The feature to prepare an execution board for.
    pub feature: String,
    /// Path to the execution graph JSON — workstreams with
    /// `id`/`title`/`operations`/`depends_on`/`write_contract`. Resolved
    /// against the current directory.
    pub graph_json: String,
}

/// What `ivar feature execute` did.
#[derive(Debug, Clone, Serialize)]
pub struct PrepareOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the board belongs to.
    pub feature: FeatureName,
    /// The board that was prepared.
    pub board: ExecutionBoard,
    /// Where the board was written.
    pub board_path: Utf8PathBuf,
}

impl WriteHuman for PrepareOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let workstreams = self.board.graph.workstreams.len();
        let noun = if workstreams == 1 {
            "workstream"
        } else {
            "workstreams"
        };
        writeln!(
            w,
            "Prepared execution board for `{}` ({workstreams} {noun}, {}) at {}",
            self.feature, self.board.status, self.board_path
        )
    }
}

/// The shape of the graph JSON `--graph-json` points at: workstreams as
/// authored, with no execution state. `status` is added when the board is
/// prepared, and `plan_fingerprint` is derived from `plan.md`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphFile {
    workstreams: Vec<GraphWorkstream>,
}

/// One workstream as authored in the graph JSON.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphWorkstream {
    id: String,
    title: String,
    operations: Vec<String>,
    depends_on: Vec<String>,
    write_contract: Vec<String>,
}

impl From<GraphWorkstream> for WorkstreamDef {
    fn from(workstream: GraphWorkstream) -> Self {
        Self {
            id: workstream.id,
            title: workstream.title,
            operations: workstream.operations,
            depends_on: workstream.depends_on,
            write_contract: workstream.write_contract,
            status: WorkstreamStatus::Waiting,
            provider: None,
            agent: None,
        }
    }
}

/// Prepare an execution board for `input.feature`.
///
/// Blocked when the feature does not exist, the feature's plan has not been
/// written, the graph file is missing or unparseable, or a board already
/// exists — an existing board carries a journal that overwriting would
/// destroy.
pub fn prepare(ctx: &Ctx, input: PrepareInput) -> Outcome<PrepareOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;
    let graph_path = ctx.resolve(Utf8Path::new(&input.graph_json));

    require_feature(&layout, &feature)?;
    require_no_board(&layout, &feature)?;

    let plan_fingerprint = plan_fingerprint(&layout, &feature)?;
    let workstreams = read_workstreams(&graph_path)?;

    let mut board = ExecutionBoard::new(ExecutionGraph {
        plan_fingerprint,
        workstreams,
    });
    board.push_journal(JournalEntry::new(
        "board",
        "prepared",
        format!("Execution board prepared from {}", graph_path),
    ));

    board.write(&layout, &feature)?;

    let board_path = feature::board_path(&layout, &feature);
    Ok(Report::new(PrepareOutcome {
        root: layout.root().to_path_buf(),
        feature,
        board_path,
        board,
    }))
}

/// Block when the feature does not exist — execution boards belong to
/// features.
fn require_feature(layout: &Layout, feature: &FeatureName) -> Result<(), Failure> {
    if fs::is_dir(&layout.feature_dir(feature))? {
        return Ok(());
    }
    Err(Failure::blocked(
        "execute.feature_not_found",
        format!("feature `{feature}` does not exist"),
    )
    .expected("an existing feature to prepare an execution board for")
    .actual(format!("`{feature}` has no feature directory"))
    .fix(FixAction::safe(
        "feature.create_first",
        format!("Create the feature first with `ivar feature create {feature}`."),
    )))
}

/// Block when the feature already has a board. Re-preparing would overwrite
/// the journal, so it takes a deliberate deletion instead.
fn require_no_board(layout: &Layout, feature: &FeatureName) -> Result<(), Failure> {
    if ExecutionBoard::read(layout, feature)?.is_none() {
        return Ok(());
    }
    let path = feature::board_path(layout, feature);
    Err(Failure::blocked(
        "execute.board_already_exists",
        format!("`{path}` already holds an execution board for `{feature}`"),
    )
    .expected("a feature with no execution board yet")
    .actual("board.json already exists — re-preparing would destroy its journal")
    .fix(FixAction::safe(
        "execute.delete_board",
        format!("Delete `{path}` deliberately, then prepare again from a fresh graph."),
    )))
}

/// SHA-256 of `plans/<feature>/plan.md` — the fingerprint that ties the
/// graph to the plan revision it was derived from.
fn plan_fingerprint(layout: &Layout, feature: &FeatureName) -> Result<String, Failure> {
    let plan = layout.plan_dir(feature).join("plan.md");
    if !fs::is_file(&plan)? {
        return Err(
            Failure::blocked("execute.plan_missing", format!("`{}` does not exist", plan))
                .expected("the feature's plan to have been written")
                .actual("no plan.md under the feature's plan directory")
                .fix(FixAction::safe(
                    "plan.create_first",
                    format!("Scaffold the plan first: `ivar plan create {feature}`."),
                )),
        );
    }
    Ok(hash::file(&plan)?)
}

/// Parse the graph JSON at `path` into the graph's workstreams. A missing
/// file is blocked; unparseable JSON fails with the path and parse position
/// from `infra::json`.
fn read_workstreams(path: &Utf8Path) -> Result<Vec<WorkstreamDef>, Failure> {
    let file: GraphFile = json::read(path)?.ok_or_else(|| {
        Failure::blocked("execute.graph_missing", format!("`{path}` does not exist"))
            .expected("an execution graph JSON file at the given path")
            .actual("no such file")
            .fix(FixAction::safe(
                "execute.provide_graph",
                "Point --graph-json at a file describing the plan's workstreams.",
            ))
    })?;
    Ok(file
        .workstreams
        .into_iter()
        .map(WorkstreamDef::from)
        .collect())
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
    use crate::action::feature::create::{
        self as feature_create, CreateInput as FeatureCreateInput,
    };
    use crate::action::hall::{self, InitInput};
    use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
    use crate::domain::feature::ExecutionStatus;
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

    /// Write the graph JSON into the hall and return its path.
    fn graph_file(root: &Utf8PathBuf) -> Utf8PathBuf {
        let path = root.join("graph.json");
        fs::write_text(&path, GRAPH_JSON).unwrap();
        path
    }

    /// The board read back off disk — the real file, not the in-memory value
    /// the action returned.
    fn persisted(root: &Utf8PathBuf) -> ExecutionBoard {
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        ExecutionBoard::read(&layout, &feature).unwrap().unwrap()
    }

    #[test]
    fn prepare_creates_a_board_from_the_graph_json() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        let graph = graph_file(&root);

        let report = prepare(
            &ctx,
            PrepareInput {
                feature: "checkout".to_owned(),
                graph_json: graph.to_string(),
            },
        )
        .unwrap();

        assert!(report.is_clean());
        assert_eq!(report.value.feature.as_str(), "checkout");
        assert_eq!(report.value.board.status, ExecutionStatus::Pending);
        assert_eq!(report.value.board.graph.workstreams.len(), 2);
        assert_eq!(report.value.board.graph.workstreams[1].id, "ws-board");
        assert_eq!(
            report.value.board.graph.workstreams[1].depends_on,
            vec!["ws-gates".to_owned()]
        );
        // Execution state is stamped by prepare, not read from the file.
        for workstream in &report.value.board.graph.workstreams {
            assert_eq!(workstream.status, WorkstreamStatus::Waiting);
        }
        // The graph is tied to the plan's current content.
        assert_eq!(
            report.value.board.graph.plan_fingerprint,
            hash::file(&root.join("plans/checkout/plan.md")).unwrap()
        );
        // The journal opens with the prepared event.
        assert_eq!(report.value.board.journal.len(), 1);
        assert_eq!(report.value.board.journal[0].kind, "prepared");
        assert_eq!(
            report.value.board_path,
            root.join(".ivar/features/checkout/execution/board.json")
        );

        // And it is persisted at the documented path.
        let on_disk = persisted(&root);
        assert_eq!(on_disk, report.value.board);
        assert!(fs::is_file(&root.join(".ivar/features/checkout/execution/board.json")).unwrap());
    }

    #[test]
    fn prepare_is_blocked_for_a_missing_feature() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        let graph = graph_file(&root);

        let failure = prepare(
            &ctx,
            PrepareInput {
                feature: "ghost".to_owned(),
                graph_json: graph.to_string(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "execute.feature_not_found");
    }

    #[test]
    fn prepare_is_blocked_when_the_plan_is_missing() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        let graph = graph_file(&root);
        fs::remove_path(&root.join("plans/checkout/plan.md")).unwrap();

        let failure = prepare(
            &ctx,
            PrepareInput {
                feature: "checkout".to_owned(),
                graph_json: graph.to_string(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "execute.plan_missing");
    }

    #[test]
    fn prepare_is_blocked_for_a_missing_graph_file() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());

        let failure = prepare(
            &ctx,
            PrepareInput {
                feature: "checkout".to_owned(),
                graph_json: "does-not-exist.json".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "execute.graph_missing");
    }

    #[test]
    fn prepare_is_blocked_for_unparseable_graph_json() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        let path = root.join("bad-graph.json");
        fs::write_text(&path, "{ not json").unwrap();

        let failure = prepare(
            &ctx,
            PrepareInput {
                feature: "checkout".to_owned(),
                graph_json: path.to_string(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Failed);
        assert_eq!(failure.code, "json.parse_failed");
    }

    #[test]
    fn prepare_is_blocked_when_a_board_already_exists() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        let graph = graph_file(&root);
        prepare(
            &ctx,
            PrepareInput {
                feature: "checkout".to_owned(),
                graph_json: graph.to_string(),
            },
        )
        .unwrap();

        let failure = prepare(
            &ctx,
            PrepareInput {
                feature: "checkout".to_owned(),
                graph_json: graph.to_string(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "execute.board_already_exists");
        assert!(failure.fix_actions[0].safe);
    }

    #[test]
    fn the_human_surface_names_the_feature_workstreams_and_board_path() {
        let outcome = PrepareOutcome {
            root: Utf8PathBuf::from("/hall"),
            feature: FeatureName::new("checkout").unwrap(),
            board_path: Utf8PathBuf::from("/hall/.ivar/features/checkout/execution/board.json"),
            board: ExecutionBoard::new(ExecutionGraph {
                plan_fingerprint: "abc".to_owned(),
                workstreams: vec![WorkstreamDef {
                    id: "ws1".to_owned(),
                    title: "WS one".to_owned(),
                    operations: vec!["op1".to_owned()],
                    depends_on: Vec::new(),
                    write_contract: Vec::new(),
                    status: WorkstreamStatus::Waiting,
                    provider: None,
                    agent: None,
                }],
            }),
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Prepared execution board for `checkout` (1 workstream, pending) at \
             /hall/.ivar/features/checkout/execution/board.json\n"
        );
    }
}
