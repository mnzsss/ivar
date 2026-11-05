#![allow(clippy::unwrap_used)]

use super::*;
use crate::store::memory::db::MemoryDb;

#[test]
fn the_leader_reindexes_on_its_own_five_second_clock() {
    let start = Instant::now();
    let mut last = None;
    assert!(reindex_due(&mut last, start));
    assert!(!reindex_due(&mut last, start + Duration::from_secs(4)));
    assert!(reindex_due(&mut last, start + Duration::from_secs(5)));
}

fn spooled_line() -> String {
    serde_json::json!({
        "event_id": "a", "source_event": "session.prompted",
        "occurred_at": "2026-11-04T10:00:00.000000000Z", "session_id": "s1",
        "native_session_id": "n1", "provider": "claude-code", "ivar_version": "0.19.0",
        "feature": "", "parent_feature": "", "repo": "", "origin": "user",
        "importance": 5, "attempt_id": "", "body": "one"
    })
    .to_string()
}

#[test]
fn a_failing_reindex_does_not_stop_the_drain_in_the_same_tick() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    {
        let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
        db.conn().execute_batch("DROP TABLE pages_fts").unwrap();
    }
    let page = layout.root().join("docs/memory/gotcha/a.md");
    std::fs::create_dir_all(page.parent().unwrap()).unwrap();
    std::fs::write(page, "# A\n").unwrap();
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("s1.jsonl"),
        format!("{}\n", spooled_line()),
    )
    .unwrap();
    let mut clock = MemoryClock::new(Instant::now());

    clock.tick(&layout, Instant::now() + DRAIN_EVERY);

    assert!(reindex::read_state(&layout).last_error.is_some());
    let drained: i64 = MemoryDb::open(layout.memory_db().as_std_path())
        .unwrap()
        .conn()
        .query_row("SELECT count(*) FROM observations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(drained, 1);
}
