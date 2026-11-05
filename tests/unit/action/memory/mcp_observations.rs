#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use serde_json::{Value, json};

use super::*;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;
use crate::test_support::seeded_hall;

fn seed(layout: &Layout, rows: &[(&str, &str, &str, &str)]) {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    for (n, (source_event, session, feature, body)) in rows.iter().enumerate() {
        db.conn()
            .execute(
                "INSERT INTO observations (event_id, source_event, occurred_at, session_id, native_session_id, provider, ivar_version, feature, importance, body) VALUES (?1, ?2, ?3, ?4, ?4, 'claude-code', '0', ?5, 4, ?6)",
                rusqlite::params![format!("e{n}"), source_event, format!("2026-11-05T10:00:{n:03}Z"), session, feature, body],
            )
            .unwrap();
    }
}

fn call_json(layout: &Layout, name: &str, args: Value) -> Value {
    serde_json::from_str(&call(layout, name, &args).unwrap()).unwrap()
}

#[test]
fn memory_recent_returns_newest_first_and_filters_by_prefix_and_feature() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    seed(
        &layout,
        &[
            ("session.prompted", "s1", "alpha", "first"),
            ("delivery.pr", "s1", "alpha", "second"),
            ("session.tool", "s2", "beta", "third"),
        ],
    );

    let all = call_json(&layout, "memory_recent", json!({}));
    assert_eq!(all["observations"][0]["body"], "third");

    let delivery = call_json(
        &layout,
        "memory_recent",
        json!({"source_event": "delivery."}),
    );
    assert_eq!(delivery["observations"].as_array().unwrap().len(), 1);

    let beta = call_json(
        &layout,
        "memory_recent",
        json!({"feature": "beta", "limit": 1}),
    );
    assert_eq!(beta["observations"][0]["body"], "third");
}

#[test]
fn memory_recent_caps_the_limit_at_one_hundred() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let bodies: Vec<String> = (0..150).map(|n| format!("body {n}")).collect();
    let rows: Vec<(&str, &str, &str, &str)> = bodies
        .iter()
        .map(|body| ("session.tool", "s1", "alpha", body.as_str()))
        .collect();
    seed(&layout, &rows);

    let out = call_json(&layout, "memory_recent", json!({"limit": 5000}));

    assert_eq!(out["observations"].as_array().unwrap().len(), 100);
}

#[test]
fn memory_read_session_observations_pages_by_cursor() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    seed(
        &layout,
        &[
            ("session.prompted", "s1", "", "one"),
            ("session.tool", "s1", "", "two"),
            ("session.tool", "s1", "", "three"),
            ("session.tool", "s2", "", "other"),
        ],
    );

    let args = json!({"session": "s1", "limit": 2});
    let page_one = call_json(&layout, "memory_read_session_observations", args);
    assert_eq!(page_one["observations"].as_array().unwrap().len(), 2);
    let cursor = page_one["next_cursor"].clone();
    let args = json!({"session": "s1", "limit": 2, "cursor": cursor});
    let page_two = call_json(&layout, "memory_read_session_observations", args);
    assert_eq!(page_two["observations"][0]["body"], "three");
    assert!(page_two["next_cursor"].is_null());
}

#[test]
fn memory_read_session_observations_needs_a_session() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    assert!(call(&layout, "memory_read_session_observations", &json!({})).is_err());
}
