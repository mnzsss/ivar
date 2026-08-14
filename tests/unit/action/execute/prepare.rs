//! Unit tests for `crate::action::execute::prepare`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
use crate::domain::feature::ExecutionStatus;
use crate::error::Status;
use crate::test_support::{hall_root, utf8_temp_dir};

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

/// A plan that backs `GRAPH_JSON` — every workstream id under `## Operations`
/// with the operations it owns, and an entry for each under
/// `## Operation details`. `prepare` refuses a graph the plan cannot back, so
/// the scaffolded plan `plan create` writes is no longer enough to seed with.
const PLAN_TEXT: &str = "# Plan\n\
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
    write_contract:\n\
    - src/action/execute\n\
    \n\
    ## Operation details\n\
    \n\
    **add-gate-types** — Add the approval gate types.\n\
    \n\
    **wire-approve** — Wire the approve verb to the gates.\n\
    \n\
    **add-board-types** — Add the execution board types.\n\
    \n\
    **store-board** — Persist the board under the feature.\n";

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
            base: None,
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
    fs::write_text(&root.join("plans/checkout/plan.md"), PLAN_TEXT).unwrap();
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
    assert_eq!(report.value.board.status, ExecutionStatus::AwaitingApproval);
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
        board: {
            let mut board = ExecutionBoard::new(ExecutionGraph {
                plan_fingerprint: "abc".to_owned(),
                workstreams: vec![WorkstreamDef {
                    id: "ws1".to_owned(),
                    title: "WS one".to_owned(),
                    operations: vec!["op1".to_owned()],
                    depends_on: Vec::new(),
                    write_contract: Vec::new(),
                    status: WorkstreamStatus::Waiting,
                    provider: None,
                    model: None,
                    agent: None,
                }],
            });
            // `prepare` leaves the board awaiting approval — mirror that
            // in the outcome the human surface renders.
            board.set_status(ExecutionStatus::AwaitingApproval);
            board
        },
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Prepared execution board for `checkout` (1 workstream, awaiting_approval) at \
         /hall/.ivar/features/checkout/execution/board.json\n"
    );
}

// --- provider/model/agent through the graph -----------------------------

/// Pinned directly: a graph carrying only the five original
/// keys (`id`, `title`, `operations`, `depends_on`, `write_contract`)
/// must still parse, and the three new fields must come out `None` — not
/// refused, not defaulted to something else.
#[test]
fn a_graph_with_only_the_five_original_fields_parses_with_the_new_fields_none() {
    let (_guard, root) = utf8_temp_dir();
    let path = root.join("graph.json");
    fs::write_text(&path, GRAPH_JSON).unwrap();

    let workstreams = read_workstreams(&path).unwrap();

    assert_eq!(workstreams.len(), 2);
    for workstream in &workstreams {
        assert_eq!(workstream.provider, None);
        assert_eq!(workstream.model, None);
        assert_eq!(workstream.agent, None);
    }
}

/// A graph that does author `provider`, `model` and `agent` carries all
/// three through to the `WorkstreamDef` — the gap this operation closes.
#[test]
fn a_graph_authoring_provider_model_and_agent_carries_all_three_through() {
    let (_guard, root) = utf8_temp_dir();
    let path = root.join("graph.json");
    fs::write_text(
        &path,
        r#"{
            "workstreams": [
                {
                    "id": "ws-exec",
                    "title": "Executor",
                    "operations": ["run"],
                    "depends_on": [],
                    "write_contract": ["src/action/execute/tick.rs"],
                    "provider": "claude-code",
                    "model": "claude-opus-4",
                    "agent": "implementer"
                }
            ]
        }"#,
    )
    .unwrap();

    let workstreams = read_workstreams(&path).unwrap();

    assert_eq!(workstreams.len(), 1);
    assert_eq!(workstreams[0].provider, Some(Provider::ClaudeCode));
    assert_eq!(workstreams[0].model.as_deref(), Some("claude-opus-4"));
    assert_eq!(workstreams[0].agent.as_deref(), Some("implementer"));
}

/// An unknown provider id is a clear refusal, not a silent `None` — it
/// must parse into `Provider` and fail naming the bad id and the valid
/// options, the same message `--provider` itself would give.
#[test]
fn a_graph_naming_an_unknown_provider_is_refused_clearly() {
    let (_guard, root) = utf8_temp_dir();
    let path = root.join("graph.json");
    fs::write_text(
        &path,
        r#"{
            "workstreams": [
                {
                    "id": "ws-exec",
                    "title": "Executor",
                    "operations": ["run"],
                    "depends_on": [],
                    "write_contract": ["src/"],
                    "provider": "not-a-provider"
                }
            ]
        }"#,
    )
    .unwrap();

    let failure = read_workstreams(&path).unwrap_err();

    assert_eq!(failure.status, Status::Failed);
    assert_eq!(failure.code, "json.parse_failed");
    assert!(
        failure.what.contains("not-a-provider"),
        "must name the rejected id: {}",
        failure.what
    );
    assert!(
        failure.what.contains("claude-code") && failure.what.contains("opencode"),
        "must name the valid options: {}",
        failure.what
    );
}

/// A graph claiming an operation the plan does not document is refused here,
/// not three commands later.
///
/// `tick` already refused it, but `tick` runs after a human approved the
/// graph, after the smart fetch, against a live board — and by then the plan
/// gate is closed, so correcting the plan means a replan. Refused at
/// `prepare`, no board exists yet and the fix is to edit `plan.md`.
#[test]
fn prepare_is_blocked_when_the_plan_does_not_document_a_claimed_operation() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    // The plan documents `wire-approve`; the graph below claims `wire-reject`.
    let path = root.join("graph.json");
    fs::write_text(
        &path,
        r#"{
            "workstreams": [
                {
                    "id": "ws-gates",
                    "title": "Approval gates",
                    "operations": ["add-gate-types", "wire-reject"],
                    "depends_on": [],
                    "write_contract": ["src/domain/feature.rs"]
                }
            ]
        }"#,
    )
    .unwrap();

    let failure = prepare(
        &ctx,
        PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: path.to_string(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "execute.operation_missing_from_plan");
    assert!(failure.what.contains("wire-reject"));

    // And no board was written — a refused prepare leaves nothing behind.
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    assert!(ExecutionBoard::read(&layout, &feature).unwrap().is_none());
}

/// The same refusal for an id the plan lists but never describes: the
/// workstream heading names it, `Operation details` does not.
#[test]
fn prepare_is_blocked_when_an_operation_has_no_entry_to_describe_it() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    fs::write_text(
        &root.join("plans/checkout/plan.md"),
        "# Plan\n\
         \n\
         ## Operations\n\
         \n\
         ### ws-gates\n\
         - add-gate-types\n\
         write_contract:\n\
         - src/domain/feature.rs\n\
         \n\
         ## Operation details\n\
         \n\
         Nothing describes add-gate-types.\n",
    )
    .unwrap();
    let path = root.join("graph.json");
    fs::write_text(
        &path,
        r#"{
            "workstreams": [
                {
                    "id": "ws-gates",
                    "title": "Approval gates",
                    "operations": ["add-gate-types"],
                    "depends_on": [],
                    "write_contract": ["src/domain/feature.rs"]
                }
            ]
        }"#,
    )
    .unwrap();

    let failure = prepare(
        &ctx,
        PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: path.to_string(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert!(failure.what.contains("add-gate-types"));
}
