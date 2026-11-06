#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

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
    assert!(read_state(&layout).dropped_last_epoch_secs.is_some());
}

#[test]
fn a_drain_with_nothing_dropped_leaves_the_last_drop_time_alone() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();

    drain(&layout).unwrap();

    assert_eq!(read_state(&layout).dropped_last_epoch_secs, None);
}

#[test]
fn a_state_file_from_an_older_ivar_still_parses() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("_drain.json"),
        r#"{"drained_total": 7}"#,
    )
    .unwrap();

    let state = read_state(&layout);

    assert_eq!(state.drained_total, 7);
    assert_eq!(state.dropped_last_epoch_secs, None);
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

#[test]
fn drops_survive_a_drain_whose_state_cannot_be_written_and_are_counted_by_the_next_one() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let dir = layout.memory_spool_dir();
    std::fs::create_dir_all(dir.join(STATE_FILE)).unwrap();
    std::fs::write(dir.join("s1.dropped"), "xxx").unwrap();

    assert!(drain(&layout).is_err());

    assert_eq!(crate::store::memory::spool::dropped_count(&dir), 3);
    std::fs::remove_dir(dir.join(STATE_FILE)).unwrap();
    drain(&layout).unwrap();
    assert_eq!(read_state(&layout).dropped_total, 3);
    assert_eq!(crate::store::memory::spool::dropped_count(&dir), 0);
}

#[test]
fn a_drop_recorded_while_a_drain_acknowledges_is_kept_for_the_next_drain() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let dir = Layout::at(root).memory_spool_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("s1.dropped"), "xx").unwrap();

    let acknowledgement = begin_acknowledgement(&dir);
    std::fs::write(dir.join("s1.dropped"), "x").unwrap();
    acknowledgement.finish();

    assert_eq!(acknowledgement.count, 2);
    assert_eq!(std::fs::read(dir.join("s1.dropped")).unwrap(), b"x");
}

#[test]
fn an_acknowledgement_left_by_a_crash_is_folded_into_the_next_drain() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let dir = layout.memory_spool_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("s1.dropped.17.ack"), "xx").unwrap();
    std::fs::write(dir.join("s1.dropped"), "x").unwrap();

    drain(&layout).unwrap();

    assert_eq!(read_state(&layout).dropped_total, 3);
    assert!(!dir.join("s1.dropped.17.ack").exists());
    assert!(!dir.join("s1.dropped").exists());
}

fn line_with(event_id: &str, source_event: &str, origin: &str, body: &str) -> String {
    let mut value: serde_json::Value = serde_json::from_str(&line(event_id, body)).unwrap();
    value["source_event"] = source_event.into();
    value["origin"] = origin.into();
    value["feature"] = "login".into();
    value.to_string()
}

fn fingerprints(layout: &Layout) -> Vec<(String, String, String, String)> {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT o.event_id, f.feature, f.day, f.normalized
               FROM request_fingerprints f JOIN observations o ON o.id = f.observation_id
              ORDER BY f.id",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn a_user_prompt_gets_one_fingerprint_and_a_harness_prompt_none() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("s1.jsonl"),
        [
            line_with(
                "a",
                "session.prompted",
                "user",
                "Please fix issue MNZS-458 in /srv/a.rs",
            ),
            line_with(
                "b",
                "session.prompted",
                "harness",
                "<system-reminder>x</system-reminder>",
            ),
            line_with("c", "session.tool", "", "{}"),
            line_with("d", "session.started", "", "{}"),
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();

    drain(&layout).unwrap();

    assert_eq!(
        fingerprints(&layout),
        vec![(
            "a".to_owned(),
            "login".to_owned(),
            "2026-11-04".to_owned(),
            "fix issue <id> <path>".to_owned()
        )]
    );
}

#[test]
fn replaying_a_user_prompt_does_not_fingerprint_it_twice() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    let spool = layout.memory_spool_dir().join("s1.jsonl");
    std::fs::write(&spool, format!("{}\n", line("a", "run the tests"))).unwrap();
    drain(&layout).unwrap();
    std::fs::write(&spool, format!("{}\n", line("a", "run the tests"))).unwrap();
    drain(&layout).unwrap();

    assert_eq!(fingerprints(&layout).len(), 1);
}

#[test]
fn a_prompt_sanitized_before_the_drain_carries_no_secret_into_its_fingerprint() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("s1.jsonl"),
        format!(
            "{}\n",
            line("a", "deploy with sk-abcdefghijklmnopqrstuvwxyz0123456789")
        ),
    )
    .unwrap();

    drain(&layout).unwrap();

    let stored = fingerprints(&layout);
    assert!(!stored[0].3.contains("abcdefghijklmnop"), "{stored:?}");
}
