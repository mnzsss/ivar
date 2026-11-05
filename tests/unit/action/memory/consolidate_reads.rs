#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::action::memory::consolidate::consolidate_if_free;
use crate::action::memory::drain::epoch_secs_now;
use crate::action::memory::{mcp, status::status_in};
use crate::domain::session::rfc3339_from_epoch_secs;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;

fn hall() -> (tempfile::TempDir, Layout) {
    let (guard, root) = crate::test_support::seeded_hall();
    (guard, Layout::at(root))
}

fn idle_conversation(layout: &Layout, native: &str) {
    let old = rfc3339_from_epoch_secs((epoch_secs_now() - 3600) as f64);
    MemoryDb::open(layout.memory_db().as_std_path())
        .unwrap()
        .conn()
        .execute(
            "INSERT INTO observations (event_id, source_event, occurred_at, session_id, native_session_id,
              provider, ivar_version, origin, importance, body)
           VALUES (?1, 'session.prompted', ?2, '', ?1, 'omp', '0', 'user', 5, 'girafa consolidada')",
            rusqlite::params![native, old],
        )
        .unwrap();
}

fn page(layout: &Layout, native: &str) -> bool {
    layout
        .root()
        .join(format!(".ivar/memory/session/{native}.md"))
        .exists()
}

#[test]
fn memory_query_consolidates_then_finds_the_session_page() {
    let (_g, layout) = hall();
    idle_conversation(&layout, "q1");
    let text = mcp::call(
        &layout,
        "memory_query",
        &serde_json::json!({"query": "girafa"}),
    )
    .unwrap();
    assert!(page(&layout, "q1"));
    assert!(text.contains(".ivar/memory/session/q1.md"), "{text}");
}

#[test]
fn memory_recent_and_status_consolidate() {
    let (_g, layout) = hall();
    idle_conversation(&layout, "r1");
    mcp::call(&layout, "memory_recent", &serde_json::json!({})).unwrap();
    assert!(page(&layout, "r1"));
    idle_conversation(&layout, "s1");
    status_in(&layout).unwrap();
    assert!(page(&layout, "s1"));
}

#[test]
fn memory_read_session_observations_consolidates() {
    let (_g, layout) = hall();
    idle_conversation(&layout, "o1");
    mcp::call(
        &layout,
        "memory_read_session_observations",
        &serde_json::json!({"session": "o1"}),
    )
    .unwrap();
    assert!(page(&layout, "o1"));
}

#[test]
fn a_read_with_the_lock_held_answers_without_consolidating() {
    let (_g, layout) = hall();
    idle_conversation(&layout, "busy");
    let held = crate::infra::fs::lock_exclusive(&layout.memory_lock()).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = layout.clone();
    std::thread::spawn(move || {
        let report = consolidate_if_free(&reader, epoch_secs_now()).unwrap();
        tx.send(report.busy).unwrap();
    });
    let busy = rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("consolidate_if_free must not wait");
    drop(held);
    assert!(busy);
    assert!(!page(&layout, "busy"));
}
