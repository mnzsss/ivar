//! Unit tests for `crate::action::execute::prompt`.
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
use crate::action::Ctx;
use crate::action::execute::prepare::{self as prepare_action, PrepareInput};
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
use crate::error::Status;
use crate::infra::fs;
use camino::Utf8PathBuf;

const PLAN_TEXT: &str = "# Plan\n\
    \n\
    ## Operation details\n\
    \n\
    **OP-A** — Do the first thing, carefully, across two files that must stay\n\
    in sync with each other.\n\
    \n\
    **OP-B** — Do the second thing.\n\
    \n\
    ## Operations\n\
    \n\
    ### prompt-render\n\
    - OP-A\n\
    - OP-B\n\
    write_contract:\n\
    - src/action/execute/plan_ops.rs\n\
    - src/action/execute/prompt.rs\n";

/// A `WorkstreamDef` built the same way `prepare` builds one — through the
/// graph JSON, not a hand-written struct literal — so this test tracks
/// whatever fields `WorkstreamDef` actually carries.
fn seeded_workstream(id: &str, operations: &[&str], write_contract: &[&str]) -> WorkstreamDef {
    let (_guard, root) = crate::test_support::hall_root();
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
    let graph_json = format!(
        r#"{{"workstreams": [{{
            "id": "{id}",
            "title": "Prompt rendering",
            "operations": {operations},
            "depends_on": [],
            "write_contract": {write_contract}
        }}]}}"#,
        operations = serde_json::to_string(operations).unwrap(),
        write_contract = serde_json::to_string(write_contract).unwrap(),
    );
    let graph = root.join("graph.json");
    fs::write_text(&graph, &graph_json).unwrap();
    let outcome = prepare_action::prepare(
        &ctx,
        PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: graph.to_string(),
        },
    )
    .unwrap();
    outcome
        .value
        .board
        .graph
        .workstreams
        .into_iter()
        .find(|workstream| workstream.id == id)
        .unwrap()
}

#[test]
fn render_composes_owned_operations_verbatim_text_and_write_contract() {
    let workstream = seeded_workstream(
        "prompt-render",
        &["OP-A", "OP-B"],
        &[
            "src/action/execute/plan_ops.rs",
            "src/action/execute/prompt.rs",
        ],
    );

    let prompt = render(PLAN_TEXT, &workstream, &[]).unwrap();

    assert!(prompt.contains("workstream `prompt-render`"));
    assert!(prompt.contains("- OP-A"));
    assert!(prompt.contains("- OP-B"));
    assert!(prompt.contains(
        "**OP-A** — Do the first thing, carefully, across two files that must stay in sync with each other."
    ));
    assert!(prompt.contains("**OP-B** — Do the second thing."));
    assert!(prompt.contains("src/action/execute/plan_ops.rs"));
    assert!(prompt.contains("src/action/execute/prompt.rs"));
    assert!(prompt.contains("one of several workstreams"));
}

#[test]
fn render_is_blocked_when_the_workstream_s_own_heading_lacks_the_operation() {
    // The graph claims OP-C, but the plan's `### prompt-render` heading
    // only lists OP-A and OP-B.
    let workstream = seeded_workstream(
        "prompt-render",
        &["OP-A", "OP-C"],
        &["src/action/execute/prompt.rs"],
    );

    let failure = render(PLAN_TEXT, &workstream, &[]).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "execute.operation_missing_from_plan");
    assert!(failure.what.contains("OP-C"));
}

#[test]
fn render_is_blocked_when_operation_details_has_no_entry_for_the_id() {
    // OP-Z is listed under the workstream's own heading but has no
    // `**OP-Z**` paragraph in Operation details.
    let plan_missing_details = "# Plan\n\
        \n\
        ## Operation details\n\
        \n\
        **OP-A** — Do the first thing.\n\
        \n\
        ## Operations\n\
        \n\
        ### prompt-render\n\
        - OP-A\n\
        - OP-Z\n\
        write_contract:\n\
        - src/action/execute/prompt.rs\n";
    let workstream = seeded_workstream(
        "prompt-render",
        &["OP-A", "OP-Z"],
        &["src/action/execute/prompt.rs"],
    );

    let failure = render(plan_missing_details, &workstream, &[]).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "execute.operation_missing_from_plan");
    assert!(failure.what.contains("OP-Z"));
}

#[test]
fn render_is_blocked_when_the_workstream_has_no_heading_in_the_plan_at_all() {
    let workstream = seeded_workstream(
        "unlisted-workstream",
        &["OP-A"],
        &["src/action/execute/prompt.rs"],
    );

    let failure = render(PLAN_TEXT, &workstream, &[]).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "execute.operation_missing_from_plan");
}

/// A workstream that blocked on a question is relaunched from scratch, so
/// the answers it already got have to travel with it — otherwise the
/// relaunch is the same prompt that produced the question.
#[test]
fn answers_from_the_human_are_rendered_into_the_prompt() {
    let workstream = seeded_workstream("prompt-render", &["OP-A"], &["src/a"]);

    let replies = vec![
        "use the v2 endpoint".to_owned(),
        "and keep the old one behind a flag".to_owned(),
    ];
    let prompt = render(PLAN_TEXT, &workstream, &replies).unwrap();

    assert!(prompt.contains("## Answers from the human"));
    assert!(prompt.contains("1. use the v2 endpoint"));
    assert!(prompt.contains("2. and keep the old one behind a flag"));
    assert!(prompt.contains("do not ask the same question again"));
}

/// The common case — a workstream that never blocked — must not carry an
/// empty section that reads as "a human said nothing".
#[test]
fn a_workstream_with_no_replies_renders_no_answers_section() {
    let workstream = seeded_workstream("prompt-render", &["OP-A"], &["src/a"]);

    let prompt = render(PLAN_TEXT, &workstream, &[]).unwrap();

    assert!(!prompt.contains("Answers from the human"));
}
