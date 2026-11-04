#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rusqlite::Connection;

use super::*;
use crate::store::graph::db::GraphDb;
use crate::store::memory::schema::{APPLICATION_ID, MEMORY_SCHEMA_VERSION};

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
fn a_fresh_file_gets_the_memory_application_id_and_schema_version_one() {
    let (_dir, path) = temp_db();

    let db = MemoryDb::open(&path).unwrap();

    assert_eq!(db.schema_version().unwrap(), MEMORY_SCHEMA_VERSION);
    drop(db);
    assert_eq!(
        pragma_i64(&path, "application_id"),
        i64::from(APPLICATION_ID)
    );
    assert_eq!(pragma_i64(&path, "user_version"), MEMORY_SCHEMA_VERSION);
    assert_eq!(object_names(&path), vec!["memory_schema".to_owned()]);
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
        .execute_batch("UPDATE memory_schema SET version = 2; PRAGMA user_version = 2;")
        .unwrap();

    let error = MemoryDb::open(&path).expect_err("newer schema must be refused");

    assert!(matches!(
        error,
        MemoryDbError::NewerSchema {
            found: 2,
            supported: MEMORY_SCHEMA_VERSION
        }
    ));
    assert_eq!(pragma_i64(&path, "user_version"), 2);
    let inspection = MemoryDb::inspect(&path).unwrap();
    assert_eq!(inspection.schema_version, 2);
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

    assert_eq!(object_names(&path), vec!["memory_schema".to_owned()]);
    assert_eq!(pragma_i64(&path, "user_version"), MEMORY_SCHEMA_VERSION);
    assert!(!MemoryDb::inspect(&path).unwrap().polluted);
}
