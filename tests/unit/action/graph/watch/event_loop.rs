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
    let start = Instant::now();
    let mut clock = MemoryClock::new(start);

    let mut drained = 0;
    for attempt in 1..=50 {
        clock.tick(&layout, start + DRAIN_EVERY * attempt);
        drained = MemoryDb::open(layout.memory_db().as_std_path())
            .unwrap()
            .conn()
            .query_row("SELECT count(*) FROM observations", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap();
        if drained == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(reindex::read_state(&layout).last_error.is_some());
    assert_eq!(drained, 1);
}

#[test]
fn a_tick_consolidates_an_idle_conversation() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    let old = crate::domain::session::rfc3339_from_epoch_secs(
        (crate::action::memory::drain::epoch_secs_now() - 3600) as f64,
    );
    db.conn()
        .execute(
            "INSERT INTO observations (event_id, source_event, occurred_at, session_id,
                native_session_id, provider, ivar_version, origin, importance, body)
             VALUES ('e2', 'session.prompted', ?1, '', 'tick', 'omp', '0', 'user', 5, 'oi')",
            [old],
        )
        .unwrap();
    drop(db);

    MemoryClock::new(Instant::now()).tick(&layout, Instant::now());

    assert!(layout.root().join(".ivar/memory/session/tick.md").exists());
}
