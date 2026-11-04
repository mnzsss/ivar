#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::session::rfc3339_from_epoch_secs;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;

const NOW: u64 = 1_800_000_000;
const DAY: u64 = 86_400;

fn at(days_ago: u64) -> String {
    rfc3339_from_epoch_secs((NOW - days_ago * DAY) as f64)
}

fn plant(layout: &Layout, rows: &[(&str, &str, &str, u64)]) {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    for (n, (event, session, native, days_ago)) in rows.iter().enumerate() {
        db.conn()
            .execute(
                "INSERT INTO observations (event_id, source_event, occurred_at, session_id,
                    native_session_id, provider, ivar_version, importance, body)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'omp', '0', 1, '{}')",
                rusqlite::params![format!("e{n}"), event, at(*days_ago), session, native],
            )
            .unwrap();
    }
}

fn remaining(layout: &Layout) -> Vec<String> {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    let mut stmt = db
        .conn()
        .prepare("SELECT event_id FROM observations ORDER BY id")
        .unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn a_session_is_deleted_whole_once_its_last_observation_passes_thirty_days() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    plant(
        &layout,
        &[
            ("session.started", "gone", "n1", 400),
            ("session.tool", "gone", "n1", 31),
            ("session.started", "alive", "n2", 400),
            ("session.tool", "alive", "n2", 1),
        ],
    );

    let report = sweep(&layout, NOW).unwrap();

    assert_eq!(report.deleted, 2);
    assert_eq!(remaining(&layout), vec!["e2", "e3"]);
}

#[test]
fn delivery_memory_and_handoff_rows_survive_however_old_they_are() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    plant(
        &layout,
        &[
            ("delivery.pushed", "", "", 400),
            ("delivery.merged", "", "", 400),
            ("memory.reviewed", "s", "n", 400),
            ("handoff.opened", "s", "n", 400),
            ("session.ended", "s", "n", 400),
        ],
    );

    let report = sweep(&layout, NOW).unwrap();

    assert_eq!(report.deleted, 1);
    assert_eq!(remaining(&layout), vec!["e0", "e1", "e2", "e3"]);
}

#[test]
fn a_large_backlog_is_deleted_across_batches() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let rows: Vec<_> = (0..2500)
        .map(|_| ("session.tool", "old", "n", 90_u64))
        .collect();
    plant(&layout, &rows);

    assert_eq!(sweep(&layout, NOW).unwrap().deleted, 2500);
    assert!(remaining(&layout).is_empty());
}

#[test]
fn sweeping_a_hall_without_a_database_deletes_nothing_and_creates_none() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);

    let report = sweep(&layout, 4_000_000_000).unwrap();

    assert_eq!(report.deleted, 0);
    assert!(!layout.memory_db().exists());
}

#[test]
fn a_busy_memory_lock_skips_the_sweep_without_waiting() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    plant(&layout, &[("session.tool", "old", "n", 90)]);
    let _held = crate::infra::fs::lock_exclusive(&layout.memory_lock()).unwrap();

    let report = sweep(&layout, NOW).unwrap();

    assert!(report.busy);
    assert_eq!(report.deleted, 0);
    assert_eq!(remaining(&layout).len(), 1);
}

#[test]
fn the_state_file_accumulates_totals_and_records_a_failure() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    plant(
        &layout,
        &[
            ("session.tool", "a", "n", 90),
            ("session.tool", "a", "n", 91),
        ],
    );
    sweep(&layout, NOW).unwrap();
    plant_more(&layout);
    sweep(&layout, NOW).unwrap();
    let state = read_state(&layout);
    assert_eq!(state.swept_total, 3);
    assert!(state.last_sweep_at.is_some());
    assert_eq!(state.last_error, None);

    std::fs::write(layout.memory_db(), b"not a database").unwrap();
    assert!(sweep(&layout, NOW).is_err());
    assert!(read_state(&layout).last_error.is_some());
}

fn plant_more(layout: &Layout) {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    db.conn()
        .execute(
            "INSERT INTO observations (event_id, source_event, occurred_at, session_id,
                native_session_id, provider, ivar_version, importance, body)
             VALUES ('late', 'session.tool', ?1, 'b', 'n', 'omp', '0', 1, '{}')",
            [at(45)],
        )
        .unwrap();
}

#[test]
fn a_lock_that_cannot_be_taken_is_recorded_as_the_last_error() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_lock()).unwrap();

    assert!(sweep(&layout, NOW).is_err());

    let state = read_state(&layout);
    assert!(state.last_error.is_some());
    assert_eq!(state.last_sweep_at, None);
}

#[test]
fn a_busy_lock_leaves_no_error_and_no_state() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let _held = crate::infra::fs::lock_exclusive(&layout.memory_lock()).unwrap();

    assert!(sweep(&layout, NOW).unwrap().busy);

    assert_eq!(read_state(&layout), SweepState::default());
}
