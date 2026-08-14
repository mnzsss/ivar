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
    let parsed = operations_from_plan(REVISED_PLAN).unwrap();

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
    assert!(operations_from_plan("# Plan\n\nprose only\n")
        .unwrap()
        .is_empty());
}
