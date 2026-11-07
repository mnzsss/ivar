#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::store::memory::db::MemoryDb;

fn open() -> (tempfile::TempDir, std::path::PathBuf, MemoryDb) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.db");
    let db = MemoryDb::open(&path).unwrap();
    (dir, path, db)
}

#[test]
fn recording_a_path_twice_keeps_only_the_latest_hash() {
    let (_dir, path, db) = open();

    record(
        db.conn(),
        ".ivar/scripts/a.sh",
        "one",
        "2026-11-06T00:00:00Z",
    )
    .unwrap();
    record(
        db.conn(),
        ".ivar/scripts/a.sh",
        "two",
        "2026-11-06T01:00:00Z",
    )
    .unwrap();
    record(
        db.conn(),
        ".ivar/scripts/b.sh",
        "three",
        "2026-11-06T01:00:00Z",
    )
    .unwrap();

    let all = read_all(&path).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all.get(".ivar/scripts/a.sh").unwrap(), "two");
    assert_eq!(all.get(".ivar/scripts/b.sh").unwrap(), "three");
}

#[test]
fn reading_a_missing_database_is_empty_and_does_not_create_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.db");

    assert!(read_all(&path).unwrap().is_empty());
    assert!(!path.exists());
}
