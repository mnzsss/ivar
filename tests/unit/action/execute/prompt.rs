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
    // The marker is written once by the renderer. Returning it attached to the
    // text as well produced `**OP-A** — **OP-A** — Do the first thing…`, which
    // a `contains` assertion cannot see because it matches the tail.
    assert_eq!(
        prompt.matches("**OP-A**").count(),
        1,
        "the operation marker was rendered twice: {prompt}"
    );
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

// -- Operation details: the body under the marker ------------------------

/// The plan shape that flew three workstreams blind: the marker alone on its
/// line, the body in the paragraph *under* it. The parser stopped at the
/// blank line between them and handed the executor `**OP-A** — **OP-A**` —
/// the operation's name and nothing else.
#[test]
fn an_operation_whose_body_sits_under_its_marker_is_rendered() {
    const PLAN: &str = "# Plan\n\
        \n\
        ## Operation details\n\
        \n\
        **OP-A**\n\
        \n\
        Delete the backend that is gone, and every wire that fed it,\n\
        including the two callers in the TUI.\n\
        \n\
        ## Operations\n\
        \n\
        ### prompt-render\n\
        - OP-A\n\
        write_contract:\n\
        - src/action/execute/prompt.rs\n";
    let workstream = seeded_workstream(
        "prompt-render",
        &["OP-A"],
        &["src/action/execute/prompt.rs"],
    );

    let prompt = render(PLAN, &workstream, &[]).unwrap();

    assert!(
        prompt.contains(
            "**OP-A** — Delete the backend that is gone, and every wire that fed it, including the two callers in the TUI."
        ),
        "body under the marker was dropped: {prompt}"
    );
    assert!(
        !prompt.contains("**OP-A** — **OP-A**"),
        "the operation rendered as its own name: {prompt}"
    );
}

/// An entry that exists but says nothing must be refused, not rendered. The
/// gate that only checked the marker's *presence* is the gate that passed a
/// nameless prompt through twice.
#[test]
fn an_operation_entry_with_no_text_is_blocked() {
    const PLAN: &str = "# Plan\n\
        \n\
        ## Operation details\n\
        \n\
        **OP-A**\n\
        \n\
        **OP-B** — Do the second thing.\n\
        \n\
        ## Operations\n\
        \n\
        ### prompt-render\n\
        - OP-A\n\
        write_contract:\n\
        - src/action/execute/prompt.rs\n";
    let workstream = seeded_workstream(
        "prompt-render",
        &["OP-A"],
        &["src/action/execute/prompt.rs"],
    );

    let failure = render(PLAN, &workstream, &[]).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert!(failure.what.contains("OP-A"));
    assert!(
        !format!("{failure:?}").contains("Do the second thing"),
        "an empty entry borrowed the next operation's text: {failure:?}"
    );
}

/// The same emptiness at the end of the section: nothing follows the marker
/// but the next heading, so there is no text to hand anyone.
#[test]
fn an_operation_entry_with_nothing_but_a_heading_after_it_is_blocked() {
    const PLAN: &str = "# Plan\n\
        \n\
        ## Operation details\n\
        \n\
        **OP-A**\n\
        \n\
        ## Operations\n\
        \n\
        ### prompt-render\n\
        - OP-A\n\
        write_contract:\n\
        - src/action/execute/prompt.rs\n";
    let workstream = seeded_workstream(
        "prompt-render",
        &["OP-A"],
        &["src/action/execute/prompt.rs"],
    );

    let failure = render(PLAN, &workstream, &[]).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert!(failure.what.contains("OP-A"));
}

/// A description that leans on a bold token mid-paragraph — an HTTP status, a
/// constant, an emphasised word — is not the next entry. Reading it as one
/// truncated `OP-BACKEND-GONE` at "passa a responder", cutting the exact
/// discriminator the operation existed to state.
#[test]
fn a_bold_token_inside_a_description_does_not_truncate_it() {
    const PLAN: &str = "# Plan\n\
        \n\
        ## Operation details\n\
        \n\
        **OP-A**\n\
        \n\
        Delete the backend. The route passa a responder\n\
        **410** com o corpo do gone-comment, e nada mais.\n\
        \n\
        ## Operations\n\
        \n\
        ### prompt-render\n\
        - OP-A\n\
        write_contract:\n\
        - src/action/execute/prompt.rs\n";
    let workstream = seeded_workstream(
        "prompt-render",
        &["OP-A"],
        &["src/action/execute/prompt.rs"],
    );

    let prompt = render(PLAN, &workstream, &[]).unwrap();

    assert!(
        prompt.contains(
            "**OP-A** — Delete the backend. The route passa a responder **410** com o corpo do gone-comment, e nada mais."
        ),
        "the description was truncated at a bold token: {prompt}"
    );
}

/// The entry that *is* next still ends the one before it, even when the plan
/// declares it — the boundary that matters is a declared operation id, not
/// any bold text.
#[test]
fn the_next_declared_operation_still_ends_the_entry_before_it() {
    const PLAN: &str = "# Plan\n\
        \n\
        ## Operation details\n\
        \n\
        **OP-A** — Do the first thing.\n\
        **OP-B** — Do the second thing.\n\
        \n\
        ## Operations\n\
        \n\
        ### prompt-render\n\
        - OP-A\n\
        - OP-B\n\
        write_contract:\n\
        - src/action/execute/prompt.rs\n";
    let workstream = seeded_workstream(
        "prompt-render",
        &["OP-A", "OP-B"],
        &["src/action/execute/prompt.rs"],
    );

    let prompt = render(PLAN, &workstream, &[]).unwrap();

    assert!(prompt.contains("**OP-A** — Do the first thing.\n"));
    assert!(prompt.contains("**OP-B** — Do the second thing.\n"));
}
