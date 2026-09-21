#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use rusqlite::Connection;

#[test]
fn test_fresh_database_migration() {
    let conn = Connection::open_in_memory().expect("open in memory db");
    apply_pragmas(&conn, false).expect("apply pragmas");
    apply_migrations(&conn).expect("apply migrations on fresh db");

    // Verify complexity column exists in symbols table
    let mut stmt = conn
        .prepare("PRAGMA table_info(symbols);")
        .expect("prepare pragma");
    let mut rows = stmt.query([]).expect("query pragma");
    let mut found_complexity = false;
    while let Some(row) = rows.next().expect("next row") {
        let name: String = row.get(1).expect("get col name");
        if name == "complexity" {
            found_complexity = true;
            let type_str: String = row.get(2).expect("get col type");
            assert_eq!(type_str.to_uppercase(), "INTEGER");
        }
    }
    assert!(found_complexity, "complexity column should exist");

    // Verify idx_symbols_complexity index exists
    let mut stmt_idx = conn
        .prepare("PRAGMA index_list(symbols);")
        .expect("prepare index_list");
    let mut idx_rows = stmt_idx.query([]).expect("query index_list");
    let mut found_idx = false;
    while let Some(row) = idx_rows.next().expect("next index row") {
        let name: String = row.get(1).expect("get index name");
        if name == "idx_symbols_complexity" {
            found_idx = true;
        }
    }
    assert!(found_idx, "idx_symbols_complexity index should exist");

    // Insert dummy repo and file
    conn.execute(
        "INSERT INTO repos (id, root_path, default_branch, last_indexed_commit, indexed_at) VALUES ('repo1', '/path', 'main', NULL, 12345);",
        [],
    )
    .expect("insert repo");
    conn.execute(
        "INSERT INTO files (id, repo, path, content_hash, mtime_ns, size_bytes, indexed_at) VALUES (1, 'repo1', 'src/lib.rs', 'hash1', 1, 100, 12345);",
        [],
    )
    .expect("insert file");

    // Insert a symbol with complexity
    conn.execute(
        "INSERT INTO symbols (id, file_id, repo, name, kind, scope, signature, docstring, start_line, start_col, end_line, end_col, is_exported, complexity)
         VALUES (1, 1, 'repo1', 'my_func', 'fn', NULL, 'fn my_func()', NULL, 1, 1, 5, 1, 1, 4);",
        [],
    )
    .expect("insert symbol with complexity");

    let comp: Option<i64> = conn
        .query_row("SELECT complexity FROM symbols WHERE id = 1;", [], |row| {
            row.get(0)
        })
        .expect("query complexity");
    assert_eq!(comp, Some(4));
}

#[test]
fn test_existing_v1_database_migration() {
    let conn = Connection::open_in_memory().expect("open in memory db");
    apply_pragmas(&conn, false).expect("apply pragmas");

    // Apply baseline MIGRATION_V1 directly
    conn.execute_batch(MIGRATION_V1).expect("apply V1 schema");

    // Insert V1 data (repo, file, symbol without complexity)
    conn.execute(
        "INSERT INTO repos (id, root_path, default_branch, last_indexed_commit, indexed_at) VALUES ('repo1', '/path', 'main', NULL, 12345);",
        [],
    )
    .expect("insert repo");
    conn.execute(
        "INSERT INTO files (id, repo, path, content_hash, mtime_ns, size_bytes, indexed_at) VALUES (1, 'repo1', 'src/main.rs', 'hash1', 1, 100, 12345);",
        [],
    )
    .expect("insert file");
    conn.execute(
        "INSERT INTO symbols (id, file_id, repo, name, kind, scope, signature, docstring, start_line, start_col, end_line, end_col, is_exported)
         VALUES (1, 1, 'repo1', 'old_fn', 'fn', NULL, 'fn old_fn()', NULL, 1, 1, 10, 1, 0);",
        [],
    )
    .expect("insert v1 symbol");

    // Now run apply_migrations (which should migrate V1 -> V2)
    apply_migrations(&conn).expect("apply migrations to existing v1 db");

    // Verify existing symbol has NULL complexity and is preserved
    let (name, comp): (String, Option<i64>) = conn
        .query_row(
            "SELECT name, complexity FROM symbols WHERE id = 1;",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query existing symbol");
    assert_eq!(name, "old_fn");
    assert_eq!(comp, None);

    // Insert new symbol with complexity
    conn.execute(
        "INSERT INTO symbols (id, file_id, repo, name, kind, scope, signature, docstring, start_line, start_col, end_line, end_col, is_exported, complexity)
         VALUES (2, 1, 'repo1', 'new_fn', 'fn', NULL, 'fn new_fn()', NULL, 12, 1, 20, 1, 1, 7);",
        [],
    )
    .expect("insert v2 symbol");

    let comp2: Option<i64> = conn
        .query_row("SELECT complexity FROM symbols WHERE id = 2;", [], |row| {
            row.get(0)
        })
        .expect("query new symbol complexity");
    assert_eq!(comp2, Some(7));
}

#[test]
fn name_words_split_identifiers_the_way_people_search_for_them() {
    assert_eq!(name_words("enforceSession"), "enforce session");
    assert_eq!(name_words("HTTPServer"), "http server");
    assert_eq!(name_words("get_user_by_id"), "get user by id");
    assert_eq!(name_words("GET /projects/:id"), "get projects id");
    assert_eq!(name_words("v2Api"), "v2 api");
}

#[test]
fn a_database_from_before_word_search_backfills_words_and_forces_a_reindex() {
    let conn = Connection::open_in_memory().expect("open in memory db");
    apply_pragmas(&conn, false).expect("apply pragmas");
    apply_migrations(&conn).expect("first migration");
    conn.execute_batch(
        "INSERT INTO repos (id, root_path, default_branch, last_indexed_commit, indexed_at)
             VALUES ('api', '/api', 'main', 'abc123', 0);
         INSERT INTO files (id, repo, path, content_hash, mtime_ns, size_bytes, indexed_at)
             VALUES (1, 'api', 'src/guard.ts', 'hash1', 5, 10, 0);
         INSERT INTO symbols (file_id, repo, name, kind, start_line, start_col, end_line, end_col)
             VALUES (1, 'api', 'enforceSession', 'fn', 1, 1, 3, 1);
         PRAGMA user_version = 2;",
    )
    .expect("seed a database from before word search");

    apply_migrations(&conn).expect("migrate");

    let words: String = conn
        .query_row("SELECT name_words FROM symbols", [], |row| row.get(0))
        .expect("name words");
    assert_eq!(words, "enforce session");
    let matches: i64 = conn
        .query_row(
            "SELECT count(*) FROM symbols_fts WHERE symbols_fts MATCH 'name_words : session'",
            [],
            |row| row.get(0),
        )
        .expect("word search");
    assert_eq!(matches, 1);
    let (hash, mtime, commit): (String, i64, Option<String>) = conn
        .query_row(
            "SELECT f.content_hash, f.mtime_ns, r.last_indexed_commit
             FROM files f JOIN repos r ON r.id = f.repo",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("file and repo rows");
    assert_eq!((hash.as_str(), mtime, commit), ("", -1, None));
}

#[test]
fn test_migration_idempotence() {
    let conn = Connection::open_in_memory().expect("open in memory db");
    apply_pragmas(&conn, false).expect("apply pragmas");

    // First call
    apply_migrations(&conn).expect("first migration run");
    // Second call
    apply_migrations(&conn).expect("second migration run");
    // Third call
    apply_migrations(&conn).expect("third migration run");
}

#[test]
fn a_database_at_the_search_version_gains_the_later_tables_and_the_current_version() {
    let conn = Connection::open_in_memory().expect("open in memory db");
    conn.execute_batch(MIGRATION_V1).expect("v1 tables");
    conn.execute_batch("PRAGMA user_version = 4;")
        .expect("mark as search version");

    apply_migrations(&conn).expect("migrate");

    let version: i64 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, SCHEMA_VERSION);
    conn.query_row("SELECT count(*) FROM layers", [], |row| {
        row.get::<_, i64>(0)
    })
    .expect("layers table exists");
}

#[test]
fn migrations_wait_for_the_write_lock_another_connection_holds() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("graph.db");
    let holder = Connection::open(&path).expect("open holder");
    apply_pragmas(&holder, true).expect("pragmas");
    holder
        .execute_batch("BEGIN IMMEDIATE;")
        .expect("hold write lock");

    let migrator = Connection::open(&path).expect("open migrator");
    migrator
        .busy_timeout(std::time::Duration::from_millis(50))
        .expect("busy timeout");
    let blocked = apply_migrations(&migrator);
    assert!(
        blocked.is_err(),
        "migration must not run without the write lock"
    );

    holder.execute_batch("COMMIT;").expect("release write lock");
    apply_migrations(&migrator).expect("migrate once the lock is free");
}

#[test]
fn concurrent_openers_of_an_old_database_both_end_up_migrated() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("graph.db");
    {
        let conn = Connection::open(&path).expect("seed");
        conn.execute_batch(MIGRATION_V1).expect("v1 tables");
        conn.execute_batch("PRAGMA user_version = 2;")
            .expect("old version");
    }

    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                let conn = Connection::open(&path).expect("open");
                conn.busy_timeout(std::time::Duration::from_secs(10))
                    .expect("busy timeout");
                apply_pragmas(&conn, true).expect("pragmas");
                apply_migrations(&conn).expect("migrate");
            });
        }
    });

    let conn = Connection::open(&path).expect("reopen");
    let version: i64 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, SCHEMA_VERSION);
}

#[test]
fn many_sessions_opening_an_old_database_at_once_all_succeed() {
    for _ in 0..30 {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("graph.db");
        {
            let conn = Connection::open(&path).expect("seed");
            conn.execute_batch(MIGRATION_V1).expect("v1 tables");
            conn.execute_batch("PRAGMA user_version = 2;")
                .expect("old version");
        }
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    let conn = Connection::open(&path).expect("open");
                    conn.busy_timeout(std::time::Duration::from_secs(10))
                        .expect("busy timeout");
                    apply_pragmas(&conn, true).expect("pragmas");
                    apply_migrations(&conn).expect("migrate");
                });
            }
        });
    }
}

#[test]
fn a_database_at_version_six_gains_the_usage_table() {
    let conn = Connection::open_in_memory().expect("open in memory db");
    conn.execute_batch(MIGRATION_V1).expect("v1 tables");
    conn.execute_batch("PRAGMA user_version = 6;")
        .expect("mark as v6");

    apply_migrations(&conn).expect("migrate");

    conn.query_row("SELECT count(*) FROM usage", [], |row| row.get::<_, i64>(0))
        .expect("usage table exists");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user_version");
    assert_eq!(version, SCHEMA_VERSION);
    let index_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_usage_command_source'",
            [],
            |row| row.get(0),
        )
        .expect("query index");
    assert_eq!(index_count, 1);
}

#[test]
fn a_database_at_the_pre_session_query_version_gains_the_new_columns() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    apply_pragmas(&conn, false).unwrap();
    conn.execute_batch(MIGRATION_V1).unwrap();
    conn.execute_batch(
        "CREATE TABLE usage (
            id INTEGER PRIMARY KEY,
            command TEXT NOT NULL,
            source TEXT NOT NULL,
            ts INTEGER NOT NULL,
            duration_ms INTEGER NOT NULL,
            result_count INTEGER,
            error INTEGER NOT NULL
        );
        PRAGMA user_version = 7;",
    )
    .unwrap();

    apply_migrations(&conn).unwrap();

    let mut stmt = conn.prepare("PRAGMA table_info(usage);").unwrap();
    let mut rows = stmt.query([]).unwrap();
    let mut names = Vec::new();
    while let Some(row) = rows.next().unwrap() {
        names.push(row.get::<_, String>(1).unwrap());
    }
    assert!(names.contains(&"session".to_owned()));
    assert!(names.contains(&"query".to_owned()));

    let version: i64 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
}

#[test]
fn a_database_at_the_current_version_without_graph_misses_gains_the_table() {
    let conn = Connection::open_in_memory().unwrap();
    apply_pragmas(&conn, false).unwrap();
    apply_migrations(&conn).unwrap();
    conn.execute_batch(&format!(
        "DROP TABLE graph_misses; PRAGMA user_version = {SCHEMA_VERSION};"
    ))
    .unwrap();

    apply_migrations(&conn).unwrap();

    let tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'graph_misses'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 1);
}

#[test]
fn a_database_from_a_newer_development_build_still_opens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.db");
    crate::store::graph::db::GraphDb::open(&path).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch(&format!("PRAGMA user_version = {};", SCHEMA_VERSION + 1))
        .unwrap();

    crate::store::graph::db::GraphDb::open(&path).unwrap();
    crate::store::graph::db::GraphDb::open_for_usage(&path).unwrap();
}

#[test]
fn a_database_at_the_current_version_without_the_usage_session_index_gains_it() {
    let conn = Connection::open_in_memory().unwrap();
    apply_pragmas(&conn, false).unwrap();
    apply_migrations(&conn).unwrap();
    conn.execute_batch(&format!(
        "DROP INDEX idx_usage_session_ts; PRAGMA user_version = {SCHEMA_VERSION};"
    ))
    .unwrap();

    apply_migrations(&conn).unwrap();

    let indexes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_usage_session_ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(indexes, 1);
}

#[test]
fn a_database_at_the_current_version_without_the_miss_usage_id_gains_it() {
    let conn = Connection::open_in_memory().unwrap();
    apply_pragmas(&conn, false).unwrap();
    apply_migrations(&conn).unwrap();
    conn.execute_batch(&format!(
        "ALTER TABLE graph_misses DROP COLUMN usage_id; PRAGMA user_version = {SCHEMA_VERSION};"
    ))
    .unwrap();

    apply_migrations(&conn).unwrap();

    assert!(has_column(&conn, "graph_misses", "usage_id").unwrap());
}
