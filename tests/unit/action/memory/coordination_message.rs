#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use serde_json::{Value, json};

use super::*;
use crate::action::memory::coordination::CoordinationCaller;
use crate::action::memory::events::MemoryEventScope;
use crate::domain::coordination::{MAX_TEXT_BYTES, Owner, Viewer};
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

fn hall_session(key: &str, provider: &str) -> CoordinationCaller {
    CoordinationCaller {
        owner: Owner {
            key: key.to_owned(),
            provider: provider.to_owned(),
        },
        viewer: Viewer {
            session_key: Some(key.to_owned()),
            provider: provider.to_owned(),
            feature: None,
        },
        scope: MemoryEventScope {
            session_id: key.to_owned(),
            ..MemoryEventScope::default()
        },
    }
}

fn spooled_events(layout: &Layout) -> Vec<Value> {
    std::fs::read_to_string(layout.memory_spool_dir().join("delivery.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn sent_id(layout: &Layout, sender: &CoordinationCaller, args: &Value) -> String {
    send(layout, sender, args).unwrap()["message_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn a_message_is_popped_exactly_once() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let sender = hall_session("s-send", "claude-code");
    let receiver = hall_session("s-recv", "omp");

    let id = sent_id(
        &layout,
        &sender,
        &json!({"body": "rebase on main", "subject": "api"}),
    );

    let first = pop(&layout, &receiver, &json!({})).unwrap();
    assert_eq!(first["status"], "popped");
    assert_eq!(first["message"]["id"], id);
    assert_eq!(first["message"]["body"], "rebase on main");
    assert!(first["notice"].as_str().unwrap().contains("untrusted"));

    assert_eq!(
        pop(&layout, &receiver, &json!({})).unwrap()["status"],
        "none_pending"
    );
    assert_eq!(
        pop(&layout, &receiver, &json!({"message_id": id})).unwrap()["status"],
        "already_claimed"
    );
}

#[test]
fn only_the_sender_cancels_and_omitting_the_id_clears_its_outbox() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let sender = hall_session("s-send", "claude-code");
    let other = hall_session("s-other", "claude-code");
    let id = sent_id(&layout, &sender, &json!({"body": "one"}));
    send(&layout, &sender, &json!({"body": "two"})).unwrap();

    assert_eq!(
        cancel(&layout, &other, &json!({"message_id": id})).unwrap()["cancelled"],
        0
    );
    assert_eq!(
        cancel(&layout, &sender, &json!({})).unwrap()["cancelled"],
        2
    );
    assert_eq!(
        list(&layout, &other, &json!({})).unwrap()["messages"],
        json!([])
    );
}

#[test]
fn a_message_addressed_to_a_provider_is_invisible_to_the_others() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let sender = hall_session("s-send", "claude-code");
    send(
        &layout,
        &sender,
        &json!({"body": "for omp", "to": {"provider": "omp"}}),
    )
    .unwrap();

    let claude = hall_session("s-claude", "claude-code");
    let omp = hall_session("s-omp", "omp");
    assert_eq!(
        list(&layout, &claude, &json!({})).unwrap()["messages"],
        json!([])
    );
    assert_eq!(
        list(&layout, &omp, &json!({})).unwrap()["messages"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        pop(&layout, &claude, &json!({})).unwrap()["status"],
        "none_pending"
    );
}

#[test]
fn the_outbox_lists_only_what_this_session_sent() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let sender = hall_session("s-send", "claude-code");
    let other = hall_session("s-other", "claude-code");
    send(&layout, &sender, &json!({"body": "mine"})).unwrap();
    send(&layout, &other, &json!({"body": "theirs"})).unwrap();

    let outbox = list(&layout, &sender, &json!({"box": "outbox"})).unwrap();
    let bodies: Vec<&str> = outbox["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["body"].as_str().unwrap())
        .collect();
    assert_eq!(bodies, vec!["mine"]);
}

#[test]
fn message_observations_never_carry_the_body() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let sender = hall_session("s-send", "claude-code");
    let receiver = hall_session("s-recv", "claude-code");
    send(&layout, &sender, &json!({"body": "secret-plan-xyz"})).unwrap();
    pop(&layout, &receiver, &json!({})).unwrap();

    let events = spooled_events(&layout);
    let names: Vec<&str> = events
        .iter()
        .map(|e| e["source_event"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["message.sent", "message.popped"]);
    assert!(
        events
            .iter()
            .all(|e| !e["body"].as_str().unwrap().contains("secret-plan-xyz"))
    );
    assert_eq!(events[0]["session_id"], "s-send");
}

#[test]
fn popping_an_expired_message_by_id_reports_none_pending() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let sender = hall_session("s-send", "claude-code");
    let receiver = hall_session("s-recv", "omp");
    let id = sent_id(&layout, &sender, &json!({"body": "stale"}));
    crate::store::memory::db::MemoryDb::open(layout.memory_db().as_std_path())
        .unwrap()
        .conn()
        .execute("UPDATE messages SET expires_at = 1 WHERE id = ?1", [&id])
        .unwrap();

    assert_eq!(
        pop(&layout, &receiver, &json!({"message_id": id})).unwrap()["status"],
        "none_pending"
    );

    let cancelled = sent_id(&layout, &sender, &json!({"body": "dropped"}));
    cancel(&layout, &sender, &json!({"message_id": cancelled})).unwrap();
    assert_eq!(
        pop(&layout, &receiver, &json!({"message_id": cancelled})).unwrap()["status"],
        "none_pending"
    );
}

#[test]
fn an_empty_or_oversized_body_is_refused() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let sender = hall_session("s-send", "claude-code");
    assert!(send(&layout, &sender, &json!({"body": ""})).is_err());
    let huge = "x".repeat(MAX_TEXT_BYTES + 1);
    assert!(send(&layout, &sender, &json!({"body": huge})).is_err());
    assert!(list(&layout, &sender, &json!({"box": "nope"})).is_err());
}
