#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use serde_json::json;

use super::*;
use crate::action::Ctx;
use crate::action::memory::coordination::{CoordinationCaller, handoff};
use crate::domain::coordination::{Owner, Viewer};
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

fn input(summary: &str) -> BeginCliInput {
    BeginCliInput {
        summary: summary.to_owned(),
        pending: vec!["p".to_owned()],
        decision: vec![],
        file: vec![],
        feature: None,
        hall: true,
        to_provider: None,
        to_session: None,
        session: None,
    }
}

#[test]
fn a_cli_handoff_is_owned_by_cli_and_listed() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let opened = begin_from_cli(&ctx, &input("from the terminal"))
        .unwrap()
        .value
        .0;
    let id = opened["handoff_id"].as_str().unwrap().to_owned();

    let listed = list_from_cli(&ctx, None, false).unwrap().value.0;
    let rows = listed["handoffs"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], id);
    assert_eq!(rows[0]["owner"], "cli");
}

#[test]
fn the_cli_cancels_a_handoff_any_session_owns() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let agent = CoordinationCaller {
        owner: Owner {
            key: "s-agent".to_owned(),
            provider: "claude-code".to_owned(),
        },
        viewer: Viewer {
            session_key: Some("s-agent".to_owned()),
            provider: "claude-code".to_owned(),
            feature: None,
        },
        scope: MemoryEventScope {
            session_id: "s-agent".to_owned(),
            ..MemoryEventScope::default()
        },
    };
    let id =
        handoff::begin(&layout, &agent, &json!({"summary": "agent work"})).unwrap()["handoff_id"]
            .as_str()
            .unwrap()
            .to_owned();

    let ctx = Ctx::new(root);
    assert_eq!(
        cancel_from_cli(&ctx, &id).unwrap().value.0["cancelled"],
        true
    );
    assert_eq!(
        list_from_cli(&ctx, None, false).unwrap().value.0["handoffs"],
        json!([])
    );
    assert!(
        cancel_from_cli(&ctx, &id).is_err(),
        "an already-cancelled handoff is a failure"
    );
}

#[test]
fn listing_without_a_memory_db_does_not_create_it() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let _ = std::fs::remove_file(layout.memory_db().as_std_path());

    let listed = list_from_cli(&Ctx::new(root), None, false).unwrap().value.0;

    assert_eq!(listed["handoffs"], json!([]));
    assert!(!layout.memory_db().exists());
}

#[test]
fn listing_an_unknown_feature_is_refused_like_the_mcp_list() {
    let (_guard, root) = seeded_hall();

    let failure = list_from_cli(&Ctx::new(root), Some("nope"), false).unwrap_err();

    assert!(
        failure.what.contains("feature `nope` does not exist"),
        "{}",
        failure.what
    );
}

#[test]
fn cancelling_without_a_memory_db_reports_no_open_handoff_and_does_not_create_it() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let _ = std::fs::remove_file(layout.memory_db().as_std_path());

    let failure = cancel_from_cli(&Ctx::new(root), "nope").unwrap_err();

    assert_eq!(failure.code, "memory.handoff_not_open");
    assert!(!layout.memory_db().exists());
}
