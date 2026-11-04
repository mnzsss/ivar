#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use camino::Utf8Path;
use rusqlite::Connection;

use super::*;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;
use crate::store::memory::schema::APPLICATION_ID;

fn hall() -> (tempfile::TempDir, Layout) {
    let (guard, root) = crate::test_support::canonical_temp_dir();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.ivar_dir()).unwrap();
    (guard, layout)
}

/// A legacy graph database at `.ivar/memory.db`, holding a marker row.
fn legacy(layout: &Layout) {
    let db = GraphDb::open(layout.memory_db().as_std_path()).unwrap();
    db.conn()
        .execute_batch("CREATE TABLE marker (v TEXT); INSERT INTO marker VALUES ('kept');")
        .unwrap();
}

/// A legacy database as a crashed old ivar leaves it: the marker row exists
/// only in `memory.db-wal`, never checkpointed into the main file.
fn legacy_crashed_with_wal_frames(layout: &Layout) {
    let stage = tempfile::tempdir().unwrap();
    let staged = Utf8Path::from_path(stage.path())
        .unwrap()
        .join("legacy.sqlite");
    let db = GraphDb::open(staged.as_std_path()).unwrap();
    db.conn()
        .execute_batch(
            "PRAGMA wal_autocheckpoint = 0;
             CREATE TABLE marker (v TEXT);
             INSERT INTO marker VALUES ('kept');",
        )
        .unwrap();
    for suffix in ["", "-wal", "-shm"] {
        std::fs::copy(
            format!("{staged}{suffix}"),
            format!("{}{suffix}", layout.memory_db()),
        )
        .unwrap();
    }
    drop(db);
    assert!(
        std::fs::metadata(format!("{}-wal", layout.memory_db()))
            .unwrap()
            .len()
            > 0
    );
}

fn marker(path: &Utf8Path) -> String {
    Connection::open(path.as_std_path())
        .unwrap()
        .query_row("SELECT v FROM marker", [], |row| row.get(0))
        .unwrap()
}

fn assert_split_done(layout: &Layout) {
    assert_eq!(marker(&layout.graph_db()), "kept");
    assert!(GraphDb::open(layout.graph_db().as_std_path()).is_ok());
    let memory = MemoryDb::inspect(layout.memory_db().as_std_path()).unwrap();
    assert_eq!(memory.application_id, APPLICATION_ID);
    assert_eq!(memory.schema_version, 1);
    assert!(!memory.polluted);
}

#[test]
fn the_legacy_database_moves_with_the_rows_that_were_only_in_its_wal() {
    let (_guard, layout) = hall();
    legacy_crashed_with_wal_frames(&layout);

    ensure_split(&layout).unwrap();

    assert_split_done(&layout);
    assert!(!is_legacy_graph_db(layout.memory_db().as_std_path()).unwrap());
}

#[test]
fn a_second_call_changes_nothing() {
    let (_guard, layout) = hall();
    legacy(&layout);

    ensure_split(&layout).unwrap();
    ensure_split(&layout).unwrap();

    assert_split_done(&layout);
}

#[test]
fn a_crash_after_the_journal_switch_and_before_the_rename_is_finished_next_time() {
    let (_guard, layout) = hall();
    legacy(&layout);
    let mode: String = Connection::open(layout.memory_db().as_std_path())
        .unwrap()
        .query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "delete");

    ensure_split(&layout).unwrap();

    assert_split_done(&layout);
}

#[test]
fn a_crash_after_the_rename_and_before_the_memory_database_is_finished_next_time() {
    let (_guard, layout) = hall();
    legacy(&layout);
    std::fs::rename(layout.memory_db(), layout.graph_db()).unwrap();

    ensure_split(&layout).unwrap();

    assert_split_done(&layout);
}

#[test]
fn another_open_connection_makes_the_move_back_off_without_touching_anything() {
    let (_guard, layout) = hall();
    legacy(&layout);
    // Every ivar open path runs a statement right after opening, so an old
    // ivar process always holds the WAL the way this holder does.
    let holder = GraphDb::open(layout.memory_db().as_std_path()).unwrap();
    let _: i64 = holder
        .conn()
        .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
        .unwrap();

    let error = ensure_split(&layout).expect_err("an open legacy database must not be moved");

    assert!(matches!(error, SplitError::Busy));
    assert!(!layout.graph_db().exists());
    assert!(is_legacy_graph_db(layout.memory_db().as_std_path()).unwrap());
    drop(holder);

    ensure_split(&layout).unwrap();
    assert_split_done(&layout);
}

#[test]
fn a_hall_without_any_database_gets_only_the_memory_database() {
    let (_guard, layout) = hall();

    ensure_split(&layout).unwrap();

    assert!(!layout.graph_db().exists());
    assert_eq!(
        MemoryDb::inspect(layout.memory_db().as_std_path())
            .unwrap()
            .application_id,
        APPLICATION_ID
    );
}

#[test]
fn a_memory_database_that_is_already_ours_is_never_moved() {
    let (_guard, layout) = hall();
    drop(MemoryDb::open(layout.memory_db().as_std_path()).unwrap());

    ensure_split(&layout).unwrap();

    assert!(!layout.graph_db().exists());
    assert!(!is_legacy_graph_db(layout.memory_db().as_std_path()).unwrap());
}

#[test]
fn the_move_waits_for_the_index_lock() {
    let (_guard, layout) = hall();
    legacy(&layout);
    let held = crate::infra::fs::lock_exclusive(&layout.graph_lock()).unwrap();
    let worker_layout = layout.clone();
    let worker = std::thread::spawn(move || ensure_split(&worker_layout));

    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        !layout.graph_db().exists(),
        "the move must wait for graph.lock"
    );
    drop(held);

    worker.join().unwrap().unwrap();
    assert_split_done(&layout);
}
#[test]
fn a_corrupt_memory_db_without_a_graph_db_is_left_alone() {
    let (_guard, layout) = hall();
    std::fs::write(layout.memory_db(), b"not a sqlite database").unwrap();

    ensure_split(&layout).unwrap();

    assert_eq!(
        std::fs::read(layout.memory_db()).unwrap(),
        b"not a sqlite database"
    );
    assert!(!layout.graph_db().exists());
}

#[test]
fn leftover_sidecars_after_the_rename_are_cleared_when_the_wal_is_empty() {
    let (_guard, layout) = hall();
    legacy(&layout);
    std::fs::rename(layout.memory_db(), layout.graph_db()).unwrap();
    std::fs::write(format!("{}-wal", layout.memory_db()), b"").unwrap();
    std::fs::write(format!("{}-shm", layout.memory_db()), [0_u8; 32_768]).unwrap();

    ensure_split(&layout).unwrap();

    assert_split_done(&layout);
}

#[test]
fn a_non_empty_orphan_wal_stops_before_the_memory_database_is_created() {
    let (_guard, layout) = hall();
    legacy(&layout);
    std::fs::rename(layout.memory_db(), layout.graph_db()).unwrap();
    let wal = format!("{}-wal", layout.memory_db());
    std::fs::write(&wal, b"orphan frames").unwrap();

    let error = ensure_split(&layout).expect_err("an orphan WAL must never meet a new database");

    assert!(matches!(error, SplitError::StaleWal));
    assert!(!layout.memory_db().exists());
    assert_eq!(std::fs::read(&wal).unwrap(), b"orphan frames");
}

#[test]
fn the_fast_path_takes_no_lock() {
    let (_guard, layout) = hall();
    legacy(&layout);
    ensure_split(&layout).unwrap();
    let _memory = crate::infra::fs::lock_exclusive(&layout.memory_lock()).unwrap();
    let _graph = crate::infra::fs::lock_exclusive(&layout.graph_lock()).unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker_layout = layout.clone();
    std::thread::spawn(move || sender.send(ensure_split(&worker_layout).is_ok()).unwrap());

    assert_eq!(
        receiver.recv_timeout(std::time::Duration::from_secs(2)),
        Ok(true),
        "with both databases present ensure_split must not wait for a lock"
    );
}

#[test]
fn finishing_the_legacy_move_never_creates_the_memory_database() {
    let (_guard, layout) = hall();

    finish_legacy_move(&layout).unwrap();
    assert!(!layout.memory_db().exists());

    legacy(&layout);
    finish_legacy_move(&layout).unwrap();
    assert_eq!(marker(&layout.graph_db()), "kept");
    assert!(!layout.memory_db().exists());
}

#[test]
fn a_memory_database_whose_creation_was_interrupted_is_created_afresh() {
    let (_guard, layout) = hall();
    let staged = format!("{}.new", layout.memory_db());
    std::fs::write(&staged, b"").unwrap();
    std::fs::write(format!("{staged}-journal"), b"").unwrap();

    ensure_split(&layout).unwrap();

    let memory = MemoryDb::inspect(layout.memory_db().as_std_path()).unwrap();
    assert_eq!(memory.application_id, APPLICATION_ID);
    assert!(!Utf8Path::new(&staged).exists());
    assert!(!Utf8Path::new(&format!("{staged}-journal")).exists());
    assert!(!Utf8Path::new(&format!("{staged}-wal")).exists());
}

#[test]
fn a_legacy_database_locked_by_another_connection_is_busy_not_skipped() {
    let (_guard, layout) = hall();
    legacy(&layout);
    let holder = Connection::open(layout.memory_db().as_std_path()).unwrap();
    let mode: String = holder
        .query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "delete");
    holder.execute_batch("BEGIN EXCLUSIVE;").unwrap();

    let error = ensure_split(&layout).expect_err("a locked legacy database must report busy");

    assert!(matches!(error, SplitError::Busy), "{error:?}");
    assert!(!layout.graph_db().exists());
    holder.execute_batch("ROLLBACK;").unwrap();
}
