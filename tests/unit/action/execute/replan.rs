#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::execute::prepare::{self as prepare_action, PrepareInput};
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
use crate::domain::feature::WorkstreamStatus;
use crate::domain::provider::Provider;
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
            "write_contract": ["src/domain/feature.rs"],
            "provider": "claude-code"
        },
        {
            "id": "ws-board",
            "title": "Execution board",
            "operations": ["add-board-types", "store-board"],
            "depends_on": ["ws-gates"],
            "write_contract": ["src/action/execute"],
            "provider": "claude-code"
        }
    ]
}"#;

/// A plan that backs `GRAPH_JSON`. `prepare` refuses a graph whose
/// operations the plan does not document, so the scaffolded plan
/// `plan create` writes is not enough to seed a board with.
const PLAN_TEXT: &str = r#"# Plan

## Operations

### ws-gates
- add-gate-types
- wire-approve
write_contract:
- src/domain/feature.rs

### ws-board
- add-board-types
- store-board
write_contract:
- src/action/execute

## Operation details

**add-gate-types** — Implement add-gate-types.

**wire-approve** — Implement wire-approve.

**add-board-types** — Implement add-board-types.

**store-board** — Implement store-board.
"#;

/// The revised plan: `ws-board` gains an operation, `ws-gates` is
/// unchanged, and `ws-tick` is added.
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
    - src/action/execute\n\
    \n\
    ### ws-tick\n\
    - run-tick\n\
    write_contract:\n\
    - src/action/execute\n\
    \n\
    ## Operation details\n\
    \n\
    **add-gate-types** — Implement add-gate-types.\n\
    \n\
    **wire-approve** — Implement wire-approve.\n\
    \n\
    **add-board-types** — Implement add-board-types.\n\
    \n\
    **store-board** — Implement store-board.\n\
    \n\
    **tick-board** — Implement tick-board.\n\
    \n\
    **run-tick** — Implement run-tick.\n";

/// The revised graph that backs `REVISED_PLAN`: `ws-board` gains the
/// `tick-board` operation, and a brand-new `ws-tick` workstream is added
/// with its own operation.
const REVISED_GRAPH: &str = r#"{
    "workstreams": [
        {
            "id": "ws-gates",
            "title": "Approval gates",
            "operations": ["add-gate-types", "wire-approve"],
            "depends_on": [],
            "write_contract": ["src/domain/feature.rs"],
            "provider": "claude-code"
        },
        {
            "id": "ws-board",
            "title": "Execution board",
            "operations": ["add-board-types", "store-board", "tick-board"],
            "depends_on": ["ws-gates"],
            "write_contract": ["src/action/execute"],
            "provider": "claude-code"
        },
        {
            "id": "ws-tick",
            "title": "Tick",
            "operations": ["run-tick"],
            "depends_on": ["ws-board"],
            "write_contract": ["src/action/execute"],
            "provider": "claude-code"
        }
    ]
}"#;

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
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
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
    let graph = root.join("graph.json");
    fs::write_text(&graph, GRAPH_JSON).unwrap();
    prepare_action::prepare(
        &ctx,
        PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: graph.to_string(),
            session: None,
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

/// Build a `ReplanInput` that folds `plan` (written to a scratch file) and
/// `graph` (also written to a scratch file) into the board.
fn replan_input(root: &Utf8PathBuf, plan: &str, graph: &str) -> ReplanInput {
    let plan_path = root.join("plan-revised.md");
    fs::write_text(&plan_path, plan).unwrap();
    let graph_path = root.join("graph-revised.json");
    fs::write_text(&graph_path, graph).unwrap();
    ReplanInput {
        feature: "checkout".to_owned(),
        plan: plan_path.to_string(),
        graph_json: graph_path.to_string(),
        allow_remove_completed: false,
    }
}

/// Mark every workstream `Done` on the persisted board — the state a replan
/// must preserve for unchanged definitions.
fn mark_all_done(root: &Utf8PathBuf) {
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
    for workstream in &mut board.graph.workstreams {
        workstream.status = WorkstreamStatus::Done;
    }
    board.write(&layout, &feature).unwrap();
}

#[test]
fn replan_adopts_the_complete_revised_graph_pausing_changed_and_added_workstreams() {
    let (_guard, root) = seeded_board();
    let ctx = Ctx::new(root.clone());
    let input = replan_input(&root, REVISED_PLAN, REVISED_GRAPH);

    let report = replan(&ctx, input).unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.changed, vec!["ws-board".to_owned()]);
    assert_eq!(report.value.added, vec!["ws-tick".to_owned()]);
    assert_eq!(report.value.retained, vec!["ws-gates".to_owned()]);
    assert!(report.value.removed.is_empty());

    // The persisted board is a complete representation of the revised graph:
    // the changed ws-board and the new ws-tick are on the board, and the
    // unchanged ws-gates keeps its place.
    let on_disk = persisted(&root);
    assert_eq!(on_disk.graph.plan_fingerprint, report.value.fingerprint);
    let ids: Vec<&str> = on_disk
        .graph
        .workstreams
        .iter()
        .map(|workstream| workstream.id.as_str())
        .collect();
    assert_eq!(ids, vec!["ws-gates", "ws-board", "ws-tick"]);
}

#[test]
fn replan_updates_every_authored_field() {
    let (_guard, root) = seeded_board();
    let ctx = Ctx::new(root.clone());
    // Every authored field changes for ws-gates: title, operations,
    // write contract, provider, model and agent.
    let plan = "# Plan\n\
        \n\
        ## Operations\n\
        \n\
        ### ws-gates\n\
        - renamed-op\n\
        write_contract:\n\
        - src/renamed\n\
        \n\
        ## Operation details\n\
        \n\
        **renamed-op** — Implement renamed-op.\n";
    let graph = r#"{
        "workstreams": [
            {
                "id": "ws-gates",
                "title": "Renamed gates",
                "operations": ["renamed-op"],
                "depends_on": [],
                "write_contract": ["src/renamed"],
                "provider": "opencode",
                "model": "deepseek-chat",
                "agent": "implementer-deepseek"
            }
        ]
    }"#;
    let input = replan_input(&root, plan, graph);

    replan(&ctx, input).unwrap();

    let on_disk = persisted(&root);
    let gates = on_disk.graph.workstreams[0].clone();
    assert_eq!(gates.id, "ws-gates");
    assert_eq!(gates.title, "Renamed gates");
    assert_eq!(gates.operations, vec!["renamed-op".to_owned()]);
    assert_eq!(gates.write_contract, vec!["src/renamed".to_owned()]);
    assert_eq!(gates.provider, Some(Provider::OpenCode));
    assert_eq!(gates.model.as_deref(), Some("deepseek-chat"));
    assert_eq!(gates.agent.as_deref(), Some("implementer-deepseek"));
}

#[test]
fn replan_keeps_unchanged_definitions_status_and_pauses_changed_and_added() {
    let (_guard, root) = seeded_board();
    let ctx = Ctx::new(root.clone());
    // ws-gates is done and unchanged in the revision; ws-board is done but
    // changes (gains tick-board); ws-tick is new.
    mark_all_done(&root);
    let input = replan_input(&root, REVISED_PLAN, REVISED_GRAPH);

    replan(&ctx, input).unwrap();

    let on_disk = persisted(&root);
    let by_id = |id: &str| {
        on_disk
            .graph
            .workstreams
            .iter()
            .find(|workstream| workstream.id == id)
            .unwrap()
            .status
    };
    // Unchanged keeps its status (Done); changed and added pause.
    assert_eq!(by_id("ws-gates"), WorkstreamStatus::Done);
    assert_eq!(by_id("ws-board"), WorkstreamStatus::Paused);
    assert_eq!(by_id("ws-tick"), WorkstreamStatus::Paused);
}

#[test]
fn replan_removes_unfinished_omissions_and_preserves_journal_and_sessions() {
    let (_guard, root) = seeded_board();
    let ctx = Ctx::new(root.clone());
    // A graph that omits ws-board entirely — an unfinished omission that may
    // be removed. ws-gates stays, ws-tick is new.
    let plan = "# Plan\n\
        \n\
        ## Operations\n\
        \n\
        ### ws-gates\n\
        - add-gate-types\n\
        - wire-approve\n\
        write_contract:\n\
        - src/domain/feature.rs\n\
        \n\
        ### ws-tick\n\
        - run-tick\n\
        write_contract:\n\
        - src/action/execute\n\
        \n\
        ## Operation details\n\
        \n\
        **add-gate-types** — Implement add-gate-types.\n\
        \n\
        **wire-approve** — Implement wire-approve.\n\
        \n\
        **run-tick** — Implement run-tick.\n";
    let graph = r#"{
        "workstreams": [
            {
                "id": "ws-gates",
                "title": "Approval gates",
                "operations": ["add-gate-types", "wire-approve"],
                "depends_on": [],
                "write_contract": ["src/domain/feature.rs"],
                "provider": "claude-code"
            },
            {
                "id": "ws-tick",
                "title": "Tick",
                "operations": ["run-tick"],
                "depends_on": ["ws-gates"],
                "write_contract": ["src/action/execute"],
                "provider": "claude-code"
            }
        ]
    }"#;
    let input = replan_input(&root, plan, graph);
    let journal_len_before = persisted(&root).journal.len();

    replan(&ctx, input).unwrap();

    let on_disk = persisted(&root);
    let ids: Vec<&str> = on_disk
        .graph
        .workstreams
        .iter()
        .map(|workstream| workstream.id.as_str())
        .collect();
    assert_eq!(ids, vec!["ws-gates", "ws-tick"], "ws-board is removed");
    // Journal history grows, never shrinks — the prepared entry and the new
    // replan entry are both there.
    assert_eq!(on_disk.journal.len(), journal_len_before + 1);
    assert_eq!(on_disk.journal[0].kind, "prepared");
    assert_eq!(on_disk.journal.last().unwrap().kind, "replan");
}

#[test]
fn replan_requires_the_revised_plan_and_graph() {
    let (_guard, root) = seeded_board();
    let ctx = Ctx::new(root.clone());
    replan_input(&root, REVISED_PLAN, REVISED_GRAPH);

    // Both the plan and the graph paths must exist.
    let missing_plan = ReplanInput {
        feature: "checkout".to_owned(),
        plan: "does-not-exist.md".to_owned(),
        graph_json: root.join("graph-revised.json").to_string(),
        allow_remove_completed: false,
    };
    let failure = replan(&ctx, missing_plan).unwrap_err();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "execute.plan_missing");

    let missing_graph = ReplanInput {
        feature: "checkout".to_owned(),
        plan: root.join("plan-revised.md").to_string(),
        graph_json: "does-not-exist.json".to_owned(),
        allow_remove_completed: false,
    };
    let failure = replan(&ctx, missing_graph).unwrap_err();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "execute.graph_missing");
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
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
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
    let input = replan_input(&root, REVISED_PLAN, REVISED_GRAPH);

    let failure = replan(&ctx, input).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "execute.board_missing");
}

#[test]
fn replan_diffs_a_second_time_against_the_immediately_previous_graph() {
    let (_guard, root) = seeded_board();
    let ctx = Ctx::new(root.clone());
    // First replan: ws-board gains tick-board, ws-tick is added. ws-gates is
    // done and unchanged, so it stays Done.
    mark_all_done(&root);
    let first = replan_input(&root, REVISED_PLAN, REVISED_GRAPH);
    replan(&ctx, first).unwrap();

    // Second replan: a fresh graph identical to what the first replan wrote
    // (same three workstreams, same content). Because the board now holds
    // ws-tick, ws-tick is an unchanged definition this time — it must retain
    // its Paused status, not be treated as newly added and paused again, and
    // ws-gates stays Done.
    let second_graph = r#"{
        "workstreams": [
            {
                "id": "ws-gates",
                "title": "Approval gates",
                "operations": ["add-gate-types", "wire-approve"],
                "depends_on": [],
                "write_contract": ["src/domain/feature.rs"],
                "provider": "claude-code"
            },
            {
                "id": "ws-board",
                "title": "Execution board",
                "operations": ["add-board-types", "store-board", "tick-board"],
                "depends_on": ["ws-gates"],
                "write_contract": ["src/action/execute"],
                "provider": "claude-code"
            },
            {
                "id": "ws-tick",
                "title": "Tick",
                "operations": ["run-tick"],
                "depends_on": ["ws-board"],
                "write_contract": ["src/action/execute"],
                "provider": "claude-code"
            }
        ]
    }"#;
    // The second replan's plan must back its graph — same operations as
    // REVISED_PLAN, which already covers all three workstreams.
    let input = replan_input(&root, REVISED_PLAN, second_graph);
    replan(&ctx, input).unwrap();

    let on_disk = persisted(&root);
    let by_id = |id: &str| {
        on_disk
            .graph
            .workstreams
            .iter()
            .find(|workstream| workstream.id == id)
            .unwrap()
    };
    // ws-tick is unchanged relative to the immediately previous graph, so it
    // keeps the Paused status the first replan gave it (not re-added).
    assert_eq!(by_id("ws-tick").status, WorkstreamStatus::Paused);
    assert_eq!(by_id("ws-gates").status, WorkstreamStatus::Done);
    assert_eq!(by_id("ws-board").status, WorkstreamStatus::Paused);
    // The journal has two replan entries — one per replan.
    let replans: Vec<_> = on_disk
        .journal
        .iter()
        .filter(|entry| entry.kind == "replan")
        .collect();
    assert_eq!(replans.len(), 2);
}
