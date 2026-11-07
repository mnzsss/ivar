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

    assert_eq!(db.schema_version().unwrap(), MEMORY_SCHEMA_VERSION);
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

#[test]
fn schema_three_migrates_to_four_with_anchor_and_usage_tables() {
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
        tx.execute_batch(crate::store::memory::schema::MIGRATION_V3)
            .unwrap();
        tx.execute("UPDATE memory_schema SET version = 3", [])
            .unwrap();
        tx.execute_batch("PRAGMA user_version = 3;").unwrap();
        tx.execute(
            "INSERT INTO pages (path, kind, tier, title, size, mtime_ns, content_hash, indexed_at)
             VALUES ('docs/memory/decision/a.md', 'decision', 'semantic', 'A', 1, 1, 'h', 't')",
            [],
        )
        .unwrap();
        tx.commit().unwrap();
    }

    let db = MemoryDb::open(&path).unwrap();

    assert_eq!(db.schema_version().unwrap(), MEMORY_SCHEMA_VERSION);
    let pending: i64 = db
        .conn()
        .query_row("SELECT anchors_pending FROM pages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(pending, 0);
    for table in ["page_anchors", "memory_usage", "memory_misses"] {
        db.conn()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap();
    }
    assert_eq!(object_names(&path), memory_objects());
}

#[test]
fn schema_four_migrates_to_five_with_coordination_tables_and_keeps_observations() {
    let (_dir, path) = temp_db();
    crate::test_support::memory_db_at_schema_four(camino::Utf8Path::from_path(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .execute(
            "INSERT INTO observations (event_id, source_event, occurred_at, native_session_id, provider, ivar_version, importance, body)
             VALUES ('e1', 'session.tool', '2026-11-06T00:00:00Z', 'n1', 'claude-code', '0.18.0', 2, '{}')",
            [],
        )
        .unwrap();

    let db = MemoryDb::open(&path).unwrap();

    assert_eq!(db.schema_version().unwrap(), MEMORY_SCHEMA_VERSION);
    let kept: i64 = db
        .conn()
        .query_row("SELECT count(*) FROM observations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(kept, 1);
    db.conn()
        .execute(
            "INSERT INTO handoffs (id, queue, owner_key, owner_provider, summary, created_at, expires_at)
             VALUES ('h1', '', 'cli', 'unknown', 'done', 1, 2)",
            [],
        )
        .unwrap();
    let (state, to_kind, tasks): (String, String, String) = db
        .conn()
        .query_row(
            "SELECT state, to_kind, pending_tasks FROM handoffs WHERE id = 'h1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (state.as_str(), to_kind.as_str(), tasks.as_str()),
        ("open", "", "[]")
    );
    db.conn()
        .execute(
            "INSERT INTO messages (id, queue, sender_key, sender_provider, body, created_at, expires_at)
             VALUES ('m1', 'login', 'cli', 'unknown', 'hi', 1, 2)",
            [],
        )
        .unwrap();
    let message_state: String = db
        .conn()
        .query_row("SELECT state FROM messages WHERE id = 'm1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(message_state, "pending");
    drop(db);
    assert_eq!(object_names(&path), memory_objects());
}

#[test]
fn an_older_database_gets_a_fingerprint_for_each_retained_user_prompt() {
    let (_dir, path) = temp_db();
    crate::test_support::memory_db_at_schema_four(camino::Utf8Path::from_path(&path).unwrap());
    let plant = Connection::open(&path).unwrap();
    for (event, source, origin, session, feature, at, body) in [
        (
            "e1",
            "session.prompted",
            "user",
            "s1",
            "login",
            "2026-11-04T23:59:59.000000000Z",
            "Corrija o login em /srv/app/src/a.rs #12",
        ),
        (
            "e2",
            "session.prompted",
            "user",
            "s2",
            "",
            "2026-11-05T00:00:01.000000000Z",
            "Fix the login",
        ),
        (
            "e3",
            "session.prompted",
            "harness",
            "s1",
            "login",
            "2026-11-05T10:00:00.000000000Z",
            "<system-reminder>x</system-reminder>",
        ),
        (
            "e4",
            "session.prompted",
            "",
            "s1",
            "login",
            "2026-11-05T11:00:00.000000000Z",
            "legacy row without an origin",
        ),
        (
            "e5",
            "session.tool",
            "",
            "s1",
            "login",
            "2026-11-05T12:00:00.000000000Z",
            "{}",
        ),
    ] {
        plant
            .execute(
                "INSERT INTO observations
                   (event_id, source_event, occurred_at, session_id, native_session_id, provider,
                    ivar_version, feature, origin, importance, body)
                 VALUES (?1, ?2, ?6, ?3, 'n1', 'claude-code', '0.18.0', ?4, ?5, 5, ?7)",
                rusqlite::params![event, source, session, feature, origin, at, body],
            )
            .unwrap();
    }
    drop(plant);

    let db = MemoryDb::open(&path).unwrap();

    assert_eq!(db.schema_version().unwrap(), MEMORY_SCHEMA_VERSION);
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT f.session_id, f.feature, f.day, f.normalized, o.event_id
               FROM request_fingerprints f JOIN observations o ON o.id = f.observation_id
              ORDER BY f.id",
        )
        .unwrap();
    let rows: Vec<(String, String, String, String, String)> = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    drop(stmt);
    let owned = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let flat: Vec<Vec<String>> = rows
        .iter()
        .map(|(a, b, c, d, e)| owned(&[a, b, c, d, e]))
        .collect();
    assert_eq!(
        flat,
        vec![
            owned(&[
                "s1",
                "login",
                "2026-11-04",
                "corrija login <path> #<n>",
                "e1"
            ]),
            owned(&["s2", "", "2026-11-05", "fix login", "e2"]),
        ]
    );
    let found: i64 = db
        .conn()
        .query_row(
            "SELECT count(*) FROM request_fingerprints_fts WHERE request_fingerprints_fts MATCH 'corrija'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(found, 1);
    drop(db);
    assert_eq!(object_names(&path), memory_objects());

    drop(MemoryDb::open(&path).unwrap());
    let again: i64 = Connection::open(&path)
        .unwrap()
        .query_row("SELECT count(*) FROM request_fingerprints", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(again, 2, "reopening does not backfill twice");
}

#[test]
fn schema_six_migrates_to_seven_with_the_decision_tables_and_keeps_its_rows() {
    let (_dir, path) = temp_db();
    crate::test_support::memory_db_at_schema_six(camino::Utf8Path::from_path(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .execute(
            "INSERT INTO request_fingerprints (observation_id, day, normalized, created_at)
             VALUES (1, '2026-11-01', 'kept', '2026-11-01T00:00:00Z')",
            [],
        )
        .unwrap();

    let db = MemoryDb::open(&path).unwrap();

    assert_eq!(db.schema_version().unwrap(), MEMORY_SCHEMA_VERSION);
    let kept: i64 = db
        .conn()
        .query_row("SELECT count(*) FROM request_fingerprints", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(kept, 1);
    db.conn()
        .execute(
            "INSERT INTO automation_decisions (kind, intent, until, created_at, updated_at)
             VALUES ('deferred', 'changelog', '2026-12-01', 'now', 'now')",
            [],
        )
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO automation_decision_texts (decision_id, normalized) VALUES (1, 'a')",
            [],
        )
        .unwrap();
    drop(db);
    assert_eq!(object_names(&path), memory_objects());
    assert_eq!(pragma_i64(&path, "user_version"), MEMORY_SCHEMA_VERSION);

    // A second open keeps the decisions.
    let db = MemoryDb::open(&path).unwrap();
    let decisions: i64 = db
        .conn()
        .query_row("SELECT count(*) FROM automation_decisions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(decisions, 1);
}

#[test]
fn the_decision_table_refuses_fields_that_do_not_belong_to_its_kind() {
    let (_dir, path) = temp_db();
    let db = MemoryDb::open(&path).unwrap();
    let insert = |kind: &str, form: Option<&str>, artifact: Option<&str>, until: Option<&str>| {
        db.conn().execute(
            "INSERT INTO automation_decisions (kind, intent, form, artifact, until, created_at, updated_at)
             VALUES (?1, 'x', ?2, ?3, ?4, 'now', 'now')",
            rusqlite::params![kind, form, artifact, until],
        )
    };

    assert!(insert("accepted", Some("script"), Some("a.sh"), None).is_ok());
    assert!(insert("declined", None, None, None).is_ok());
    assert!(insert("deferred", None, None, Some("2026-12-01")).is_ok());
    assert!(insert("accepted", None, None, None).is_err());
    assert!(insert("declined", Some("script"), None, None).is_err());
    assert!(insert("declined", None, None, Some("2026-12-01")).is_err());
    assert!(insert("deferred", None, None, None).is_err());
    assert!(insert("accepted", Some("tool"), Some("a"), None).is_err());
    assert!(insert("maybe", None, None, None).is_err());
}

#[test]
fn a_current_database_opens_read_only_without_changing_a_byte() {
    let (_dir, path) = temp_db();
    drop(MemoryDb::open(&path).unwrap());
    let before = std::fs::read(&path).unwrap();

    let ReadOnlyOpen::Current(conn) = open_current_read_only(&path).unwrap() else {
        panic!("a freshly migrated database is current");
    };
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM request_fingerprints", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(rows, 0);
    let write = conn.execute("DELETE FROM request_fingerprints", []);
    assert!(write.is_err(), "the connection is read-only");
    drop(conn);

    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn an_outdated_database_is_reported_and_not_migrated() {
    let (_dir, path) = temp_db();
    drop(MemoryDb::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .execute_batch("UPDATE memory_schema SET version = 3;")
        .unwrap();

    let opened = open_current_read_only(&path).unwrap();

    assert!(matches!(
        opened,
        ReadOnlyOpen::Outdated { found: 3, current } if current == MEMORY_SCHEMA_VERSION
    ));
    assert_eq!(pragma_i64(&path, "user_version"), MEMORY_SCHEMA_VERSION);
    let stored: i64 = Connection::open(&path)
        .unwrap()
        .query_row("SELECT max(version) FROM memory_schema", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stored, 3);
}

#[test]
fn a_newer_database_and_a_foreign_one_are_errors() {
    let (_dir, path) = temp_db();
    drop(MemoryDb::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .execute_batch("UPDATE memory_schema SET version = 99;")
        .unwrap();
    assert!(matches!(
        open_current_read_only(&path),
        Err(MemoryDbError::NewerSchema { found: 99, .. })
    ));

    let (_dir, foreign) = temp_db();
    drop(GraphDb::open(&foreign).unwrap());
    assert!(matches!(
        open_current_read_only(&foreign),
        Err(MemoryDbError::ForeignApplication { .. })
    ));
}

#[test]
fn an_immutable_uri_escapes_what_a_uri_cannot_carry() {
    assert_eq!(
        immutable_uri(std::path::Path::new("/tmp/a b/c?d#e%f/memory.db")),
        "file:/tmp/a%20b/c%3Fd%23e%25f/memory.db?immutable=1"
    );
}

#[test]
fn a_database_in_an_unwritable_directory_is_read_through_an_immutable_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("with space").join("mem.sqlite");
    std::fs::create_dir(path.parent().unwrap()).unwrap();
    drop(MemoryDb::open(&path).unwrap());

    let conn = open_for_reading(&path).unwrap();
    drop(conn);

    let immutable = Connection::open_with_flags(
        immutable_uri(&path),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .unwrap();
    let id: i32 = immutable
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .unwrap();
    assert_eq!(id, APPLICATION_ID, "the escaped uri names the same file");
}
