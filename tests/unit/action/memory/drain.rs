#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;

fn line(event_id: &str, body: &str) -> String {
    serde_json::json!({
        "event_id": event_id, "source_event": "session.prompted",
        "occurred_at": "2026-11-04T10:00:00.000000000Z", "session_id": "s1",
        "native_session_id": "n1", "provider": "claude-code", "ivar_version": "0.19.0",
        "feature": "", "parent_feature": "", "repo": "", "origin": "user",
        "importance": 5, "attempt_id": "", "body": body
    })
    .to_string()
}

fn rows(layout: &Layout) -> i64 {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    db.conn()
        .query_row("SELECT count(*) FROM observations", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn draining_moves_lines_into_the_table_and_removes_the_spool() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("s1.jsonl"),
        format!("{}\n{}\n", line("a", "one"), line("b", "two")),
    )
    .unwrap();

    let report = drain(&layout).unwrap();

    assert_eq!(report.inserted, 2);
    assert_eq!(rows(&layout), 2);
    assert!(!layout.memory_spool_dir().join("s1.jsonl").exists());
    assert!(read_state(&layout).last_drain_at.is_some());
}

#[test]
fn replaying_a_partially_drained_spool_does_not_duplicate_observations() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("s1.jsonl"),
        format!("{}\n", line("a", "one")),
    )
    .unwrap();
    drain(&layout).unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("s1.draining"),
        format!("{}\n{}\n", line("a", "one"), line("b", "two")),
    )
    .unwrap();

    let report = drain(&layout).unwrap();

    assert_eq!(report.inserted, 1, "only the new event is inserted");
    assert_eq!(rows(&layout), 2);
}

#[test]
fn a_drain_acknowledges_dropped_events_and_clears_their_counter() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    std::fs::write(layout.memory_spool_dir().join("s1.dropped"), "xxx").unwrap();

    drain(&layout).unwrap();

    assert!(!layout.memory_spool_dir().join("s1.dropped").exists());
    assert_eq!(read_state(&layout).dropped_total, 3);
}

#[test]
fn a_new_spool_never_overwrites_the_leftover_of_a_failed_drain() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("s1.1.draining"),
        format!("{}\n", line("old", "old")),
    )
    .unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("s1.jsonl"),
        format!("{}\n", line("new", "new")),
    )
    .unwrap();

    let report = drain(&layout).unwrap();

    assert_eq!(
        report.inserted, 2,
        "both the leftover and the new events are drained"
    );
}

#[test]
fn a_corrupt_line_is_skipped_and_never_loses_its_neighbours() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("s1.jsonl"),
        format!("{}\n{{broken\n{}\n", line("a", "one"), line("b", "two")),
    )
    .unwrap();

    let report = drain(&layout).unwrap();

    assert_eq!((report.inserted, report.skipped_lines), (2, 1));
}

#[test]
fn the_drain_re_sanitizes_because_the_spool_is_untrusted_input() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("s1.jsonl"),
        format!(
            "{}\n",
            line("a", "ghp_0123456789abcdefghijklmnopqrstuvwxyzAB")
        ),
    )
    .unwrap();

    drain(&layout).unwrap();

    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    let body: String = db
        .conn()
        .query_row("SELECT body FROM observations", [], |r| r.get(0))
        .unwrap();
    assert!(!body.contains("ghp_"), "{body}");
}

#[test]
fn a_failed_drain_leaves_the_spool_intact_and_records_the_error() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("s1.jsonl"),
        format!("{}\n", line("a", "one")),
    )
    .unwrap();
    std::fs::create_dir_all(layout.memory_db().as_std_path()).unwrap(); // a directory where the database file should be

    assert!(drain(&layout).is_err());

    let leftover = std::fs::read_dir(layout.memory_spool_dir())
        .unwrap()
        .flatten()
        .any(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext == "draining" || ext == "jsonl")
        });
    assert!(leftover, "the events must survive a failed drain");
    assert!(read_state(&layout).last_error.is_some());
}

#[test]
fn drain_one_takes_a_single_spool_file_per_call() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("s1.jsonl"),
        format!("{}\n", line("a", "one")),
    )
    .unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("s2.jsonl"),
        format!("{}\n", line("b", "two")),
    )
    .unwrap();

    let report = drain_one(&layout).unwrap();

    assert_eq!((report.files, report.inserted), (1, 1));
    assert_eq!(rows(&layout), 1);
}

#[test]
fn drain_one_returns_an_empty_report_while_another_process_holds_the_lock() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("s1.jsonl"),
        format!("{}\n", line("a", "one")),
    )
    .unwrap();
    let _held = crate::infra::fs::lock_exclusive(&layout.memory_lock()).unwrap();

    let report = drain_one(&layout).unwrap();

    assert_eq!(report, DrainReport::default());
    assert!(layout.memory_spool_dir().join("s1.jsonl").exists());
}
