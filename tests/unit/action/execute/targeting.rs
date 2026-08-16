//! Unit tests for `crate::action::execute::targeting` — provider resolution
//! merged from the graph, the plan, and the caller session.
//!
//! Physically located here but compiled inside the library crate via
//! `#[path]` so `use super::*` reaches `targeting`'s private items.
//! `targeting` has no public entry point of its own — it is declared `mod
//! targeting;` (private) in `execute/mod.rs` — so, like every external
//! caller, these tests exercise it through
//! `crate::action::execute::prepare::prepare`, the only function that calls
//! `targeting::resolve`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use super::*;
use crate::action::Ctx;
use crate::action::execute::prepare::{PrepareInput, prepare};
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
use crate::infra::fs;
use crate::test_support::{feature_session, hall_root};

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

/// A hall with a `checkout` feature and a plan already backing `GRAPH_JSON`.
///
/// Not the shared `crate::test_support::seeded_hall` — that hall has no
/// feature or plan in it. This is `prepare.rs`'s own fixture, duplicated
/// rather than shared: it seeds a plan tailored to `GRAPH_JSON`'s two
/// workstreams, which is specific to targeting resolution, not a general
/// hall fixture.
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
    (guard, root)
}

/// Write the graph JSON into the hall and return its path.
fn graph_file(root: &Utf8PathBuf) -> Utf8PathBuf {
    let path = root.join("graph.json");
    fs::write_text(&path, GRAPH_JSON).unwrap();
    path
}

// --- provider resolution from the caller session -------------------------

/// A provider-less graph with no caller session is refused with a structured
/// recovery message — `prepare` never falls back silently to the hall default.
#[test]
fn prepare_refuses_an_unresolved_provider_without_a_caller_session() {
    let (_guard, root) = seeded_hall();
    let graph = graph_file(&root);
    let failure = prepare(
        &Ctx::new(root.clone()),
        PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: graph.to_string(),
            session: None,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "execute.provider_context_missing");
    assert!(failure.what.contains("caller session"));
}

/// The graph and the plan naming different providers for the same workstream
/// is a refusal — the two artifacts must not drift silently.
#[test]
fn prepare_refuses_provider_drift_between_plan_and_graph() {
    let (_guard, root) = seeded_hall();
    let plan_path = root.join("plans/checkout/plan.md");
    let plan = fs::read_text(&plan_path).unwrap().unwrap().replacen(
        "### ws-gates\n",
        "### ws-gates\nprovider: opencode\n",
        1,
    );
    fs::write_text(&plan_path, &plan).unwrap();
    let graph = root.join("graph.json");
    let graph_text = GRAPH_JSON.replacen(
        "\"write_contract\": [\"src/domain/feature.rs\"]",
        "\"write_contract\": [\"src/domain/feature.rs\"],\n            \"provider\": \"claude-code\"",
        1,
    );
    fs::write_text(&graph, &graph_text).unwrap();
    let session = feature_session(&root, Provider::OpenCode);

    let failure = prepare(
        &Ctx::new(root),
        PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: graph.to_string(),
            session: Some(session),
        },
    )
    .unwrap_err();
    assert_eq!(failure.code, "execute.targeting_conflict");
    assert!(failure.what.contains("ws-gates"));
    assert!(failure.what.contains("opencode"));
    assert!(failure.what.contains("claude-code"));
}
