#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use serde_json::{Value, json};

use super::*;
use crate::action::memory::mcp::{MemoryCaller, call_as};
use crate::domain::name::FeatureName;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

fn spooled(layout: &Layout) -> Vec<Value> {
    std::fs::read_to_string(layout.memory_spool_dir().join("delivery.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn body(line: &Value) -> Value {
    serde_json::from_str(line["body"].as_str().unwrap()).unwrap()
}

fn scope() -> MemoryEventScope {
    MemoryEventScope {
        session_id: "s1".to_owned(),
        feature: "login".to_owned(),
        parent_feature: String::new(),
    }
}

fn caller() -> MemoryCaller {
    MemoryCaller {
        session: Some("s1".to_owned()),
        feature: Some(FeatureName::new("login").unwrap()),
        parent_feature: None,
        ..Default::default()
    }
}

#[test]
fn a_memory_event_is_spooled_with_its_scope_and_fixed_importance() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);

    record_memory_event(
        &layout,
        &scope(),
        SourceEvent::MemoryReviewed,
        &json!({"heads": {"api": "abc"}}),
    )
    .unwrap();

    let lines = spooled(&layout);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["source_event"], "memory.reviewed");
    assert_eq!(lines[0]["feature"], "login");
    assert_eq!(lines[0]["session_id"], "s1");
    assert_eq!(lines[0]["importance"], 5);
    assert_eq!(body(&lines[0])["heads"]["api"], "abc");
}

#[test]
fn a_session_event_is_refused_and_nothing_is_spooled() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);

    let error =
        record_memory_event(&layout, &scope(), SourceEvent::SessionTool, &json!({})).unwrap_err();

    assert!(error.contains("not a memory event"), "{error}");
    assert!(spooled(&layout).is_empty());
}

#[test]
fn writing_a_page_spools_one_page_written_for_the_callers_feature() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);

    let text = call_as(
        &layout,
        "memory_write_page",
        &json!({"kind": "gotcha", "title": "Locks leak", "body": "Release the lock."}),
        &caller(),
    )
    .unwrap();

    let written: Value = serde_json::from_str(&text).unwrap();
    let lines = spooled(&layout);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["source_event"], "memory.page_written");
    assert_eq!(lines[0]["feature"], "login");
    assert_eq!(
        body(&lines[0]),
        json!({"path": written["path"], "kind": "gotcha", "action": "written", "sha256": written["hash"]})
    );
}

#[test]
fn deleting_a_page_spools_a_deleted_action_with_an_empty_hash() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let text = call_as(
        &layout,
        "memory_write_page",
        &json!({"kind": "decision", "title": "Keep one lock", "body": "One lock."}),
        &caller(),
    )
    .unwrap();
    let path = serde_json::from_str::<Value>(&text).unwrap()["path"].clone();

    call_as(
        &layout,
        "memory_delete_page",
        &json!({"path": path}),
        &caller(),
    )
    .unwrap();

    let lines = spooled(&layout);
    assert_eq!(lines.len(), 2);
    assert_eq!(body(&lines[1])["action"], "deleted");
    assert_eq!(body(&lines[1])["sha256"], "");
}

#[test]
fn a_refused_write_spools_nothing() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);

    call_as(
        &layout,
        "memory_write_page",
        &json!({"kind": "session", "title": "x", "body": "y"}),
        &caller(),
    )
    .unwrap_err();

    assert!(spooled(&layout).is_empty());
}

#[test]
fn a_coordination_event_is_spooled_with_its_importance() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);

    record_memory_event(
        &layout,
        &scope(),
        SourceEvent::HandoffOpened,
        &json!({"id": "h1", "queue": "login", "owner": "s1"}),
    )
    .unwrap();
    record_memory_event(
        &layout,
        &scope(),
        SourceEvent::MessageSent,
        &json!({"id": "m1", "queue": ""}),
    )
    .unwrap();

    let lines = spooled(&layout);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["source_event"], "handoff.opened");
    assert_eq!(lines[0]["importance"], 5);
    assert_eq!(body(&lines[0])["id"], "h1");
    assert_eq!(lines[1]["source_event"], "message.sent");
    assert_eq!(lines[1]["importance"], 4);
}
