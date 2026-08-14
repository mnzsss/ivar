#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::execute::replan::{self, ReplanInput};
use crate::domain::feature::ExecutionStatus;
use crate::error::Status;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::test_support::hall_root;

use crate::action::execute::prepare::{self as prepare_action, PrepareInput};
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};

const GRAPH_JSON: &str = r#"{
    "workstreams": [
        {
            "id": "ws-a",
            "title": "A",
            "operations": ["op-a1", "op-a2"],
            "depends_on": [],
            "write_contract": ["src/a"],
            "provider": "claude-code"
        },
        {
            "id": "ws-b",
            "title": "B",
            "operations": ["op-b1"],
            "depends_on": ["ws-a"],
            "write_contract": ["src/b"],
            "provider": "claude-code"
        }
    ]
}"#;

/// A plan that backs `GRAPH_JSON`. `prepare` refuses a graph whose
/// operations the plan does not document, so the scaffolded plan
/// `plan create` writes is not enough to seed a board with.
const PLAN_TEXT: &str = r#"# Plan

## Operations

### ws-a
- op-a1
- op-a2
write_contract:
- src/a

### ws-b
- op-b1
write_contract:
- src/b

## Operation details

**op-a1** — Implement op-a1.

**op-a2** — Implement op-a2.

**op-b1** — Implement op-b1.
"#;

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
    assert_eq!(
        on_disk.status,
        ExecutionStatus::Approved,
        "a resumed board must be tickable — nothing is running, the unpaused \
         workstreams are waiting to be launched"
    );
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
