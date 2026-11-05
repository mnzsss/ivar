#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rusqlite::Connection;

use super::*;
use crate::store::graph::db::GraphDb;
use crate::store::memory::schema::{APPLICATION_ID, MEMORY_OBJECTS, MEMORY_SCHEMA_VERSION};

fn temp_db() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mem.sqlite");
    (dir, path)
}

fn pragma_i64(path: &std::path::Path, pragma: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(&format!("PRAGMA {pragma}"), [], |row| row.get(0))
        .unwrap()
}

fn memory_objects() -> Vec<String> {
    let mut names: Vec<String> = MEMORY_OBJECTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    names.sort();
    names
}

fn object_names(path: &std::path::Path) -> Vec<String> {
    let conn = Connection::open(path).unwrap();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn a_fresh_file_gets_the_memory_application_id_and_the_current_schema_version() {
    let (_dir, path) = temp_db();

    let db = MemoryDb::open(&path).unwrap();

    assert_eq!(db.schema_version().unwrap(), MEMORY_SCHEMA_VERSION);
    drop(db);
    assert_eq!(
        pragma_i64(&path, "application_id"),
        i64::from(APPLICATION_ID)
    );
    assert_eq!(pragma_i64(&path, "user_version"), MEMORY_SCHEMA_VERSION);
    assert_eq!(object_names(&path), memory_objects());
    let journal: String = Connection::open(&path)
        .unwrap()
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal, "wal");
}

#[test]
fn reopening_keeps_a_single_version_row() {
    let (_dir, path) = temp_db();
    MemoryDb::open(&path).unwrap();
    MemoryDb::open(&path).unwrap();

    let rows: i64 = Connection::open(&path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM memory_schema", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 1);
}

#[test]
fn a_newer_schema_is_refused_and_left_untouched() {
    let (_dir, path) = temp_db();
    MemoryDb::open(&path).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch(&format!(
            "UPDATE memory_schema SET version = {0}; PRAGMA user_version = {0};",
            MEMORY_SCHEMA_VERSION + 1
        ))
        .unwrap();

    let error = MemoryDb::open(&path).expect_err("newer schema must be refused");

    assert!(matches!(
        error,
        MemoryDbError::NewerSchema { found, supported }
            if found == MEMORY_SCHEMA_VERSION + 1 && supported == MEMORY_SCHEMA_VERSION
    ));
    assert_eq!(pragma_i64(&path, "user_version"), MEMORY_SCHEMA_VERSION + 1);
    let inspection = MemoryDb::inspect(&path).unwrap();
    assert_eq!(inspection.schema_version, MEMORY_SCHEMA_VERSION + 1);
}

#[test]
fn a_legacy_graph_database_is_refused_as_foreign_and_not_touched() {
    let (_dir, path) = temp_db();
    drop(GraphDb::open(&path).unwrap());

    let error = MemoryDb::open(&path).expect_err("legacy graph db is not ours");

    assert!(matches!(
        error,
        MemoryDbError::ForeignApplication { found: 0 }
    ));
    assert!(object_names(&path).contains(&"files".to_owned()));
    assert_eq!(pragma_i64(&path, "application_id"), 0);
}

#[test]
fn pollution_by_an_older_ivar_is_reported_by_inspect_and_repaired_by_open() {
    let (_dir, path) = temp_db();
    drop(MemoryDb::open(&path).unwrap());
    // An ivar older than the split opens `.ivar/memory.db` as its graph:
    // it creates every graph object and stamps user_version 11.
    drop(GraphDb::open(&path).unwrap());

    let polluted = MemoryDb::inspect(&path).unwrap();
    assert!(polluted.polluted);
    assert_eq!(polluted.application_id, APPLICATION_ID);
    assert_eq!(polluted.schema_version, MEMORY_SCHEMA_VERSION);
    assert!(object_names(&path).contains(&"symbols".to_owned()));

    drop(MemoryDb::open(&path).unwrap());

    assert_eq!(object_names(&path), memory_objects());
    assert_eq!(pragma_i64(&path, "user_version"), MEMORY_SCHEMA_VERSION);
    assert!(!MemoryDb::inspect(&path).unwrap().polluted);
}

#[test]
fn an_empty_file_left_by_an_interrupted_create_is_created_afresh() {
    let (_dir, path) = temp_db();
    std::fs::write(&path, b"").unwrap();

    drop(MemoryDb::open(&path).unwrap());

    assert_eq!(
        MemoryDb::inspect(&path).unwrap().application_id,
        APPLICATION_ID
    );
}

const OBSERVATION_OBJECTS: [&str; 5] = [
    "observations",
    "observations_session",
    "observations_source_event",
    "observations_time",
    "observations_no_update",
];

fn insert_observation(db: &MemoryDb, event_id: &str) -> rusqlite::Result<usize> {
    db.conn().execute(
        "INSERT INTO observations
           (event_id, source_event, occurred_at, native_session_id, provider, ivar_version, importance, body)
         VALUES (?1, 'session.started', '2026-11-04T10:00:00.000000000Z', 'native-1', 'claude-code', '0.19.0', 1, '{}')",
        [event_id],
    )
}

#[test]
fn reopening_keeps_every_observation_object() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.db");
    drop(MemoryDb::open(&path).unwrap());

    let db = MemoryDb::open(&path).unwrap();

    for name in OBSERVATION_OBJECTS {
        let found: i64 = db
            .conn()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(found, 1, "{name} was dropped or never created");
    }
}

#[test]
fn a_v1_database_migrates_to_v2_and_keeps_its_schema_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(&format!(
        "PRAGMA application_id = {APPLICATION_ID};
         CREATE TABLE memory_schema (version INTEGER NOT NULL);
         INSERT INTO memory_schema (version) VALUES (1);"
    ))
    .unwrap();
    drop(conn);

    let db = MemoryDb::open(&path).unwrap();

    assert_eq!(db.schema_version().unwrap(), MEMORY_SCHEMA_VERSION);
    assert_eq!(insert_observation(&db, "e1").unwrap(), 1);
    let extension: String = db
        .conn()
        .query_row("SELECT extension FROM observations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(extension, "ivar");
}

#[test]
fn observations_are_append_only_and_unique_by_event_id() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDb::open(&dir.path().join("memory.db")).unwrap();
    insert_observation(&db, "e1").unwrap();

    let update = db.conn().execute("UPDATE observations SET body = 'x'", []);
    let duplicate = insert_observation(&db, "e1");

    assert!(update.is_err(), "UPDATE must be rejected");
    assert!(duplicate.is_err(), "event_id is unique");
}

#[test]
fn schema_two_migrates_to_three_and_indexes_existing_observations() {
    let (_dir, path) = temp_db();
    {
        let mut conn = Connection::open(&path).unwrap();
        conn.execute_batch(&format!("PRAGMA application_id = {APPLICATION_ID};"))
            .unwrap();
        let tx = conn.transaction().unwrap();
        tx.execute_batch(crate::store::memory::schema::MIGRATION_V1)
            .unwrap();
        tx.execute_batch(crate::store::memory::schema::MIGRATION_V2)
            .unwrap();
        tx.execute("UPDATE memory_schema SET version = 2", [])
            .unwrap();
        tx.execute_batch("PRAGMA user_version = 2;").unwrap();
        tx.execute(
            "INSERT INTO observations (event_id, source_event, occurred_at, native_session_id, provider, ivar_version, importance, body)
             VALUES ('e1', 'delivery.pr', '2026-11-01T00:00:00Z', '', 'claude-code', '0', 4, 'opened the pull request for teardown')",
            [],
        )
        .unwrap();
        tx.commit().unwrap();
    }

    let db = MemoryDb::open(&path).unwrap();

    assert_eq!(db.schema_version().unwrap(), 3);
    let hits: i64 = db
        .conn()
        .query_row(
            "SELECT count(*) FROM observations_fts WHERE observations_fts MATCH 'teardown'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hits, 1);
    let rows: i64 = db
        .conn()
        .query_row("SELECT count(*) FROM observations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);
}

#[test]
fn reopening_keeps_both_indexes() {
    let (_dir, path) = temp_db();
    let db = MemoryDb::open(&path).unwrap();
    db.conn()
        .execute(
            "INSERT INTO observations (event_id, source_event, occurred_at, native_session_id, provider, ivar_version, importance, body)
             VALUES ('e1', 'delivery.pr', '2026-11-01T00:00:00Z', '', 'p', '0', 4, 'findable text')",
            [],
        )
        .unwrap();
    db.conn()
        .execute("INSERT INTO pages_fts (rowid, title, tags, body) VALUES (1, 'T', '', 'findable page body')", [])
        .unwrap();
    drop(db);

    let db = MemoryDb::open(&path).unwrap();

    for (table, expected) in [("observations_fts", 1), ("pages_fts", 1)] {
        let hits: i64 = db
            .conn()
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE {table} MATCH 'findable'"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, expected, "{table}");
    }
}

#[test]
fn deleting_an_observation_removes_it_from_the_search_index() {
    let (_dir, path) = temp_db();
    let db = MemoryDb::open(&path).unwrap();
    db.conn()
        .execute(
            "INSERT INTO observations (event_id, source_event, occurred_at, native_session_id, provider, ivar_version, importance, body)
             VALUES ('e1', 'session.tool', '2020-01-01T00:00:00Z', 's', 'p', '0', 2, 'swept words')",
            [],
        )
        .unwrap();
    db.conn().execute("DELETE FROM observations", []).unwrap();
    let hits: i64 = db
        .conn()
        .query_row(
            "SELECT count(*) FROM observations_fts WHERE observations_fts MATCH 'swept'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hits, 0);
}
