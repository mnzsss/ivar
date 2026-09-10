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
