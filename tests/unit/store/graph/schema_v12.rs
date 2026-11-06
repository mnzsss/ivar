#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

#[test]
fn version_twelve_purges_indexed_content_of_hall_memory_pages() {
    let conn = Connection::open_in_memory().expect("open");
    apply_pragmas(&conn, false).expect("pragmas");
    apply_migrations(&conn).expect("migrate");
    conn.execute_batch(
        "INSERT INTO repos (id, root_path, default_branch, indexed_at) VALUES ('hall', '/h', '', 0);
         INSERT INTO files (id, repo, path, content_hash, mtime_ns, size_bytes, indexed_at)
           VALUES (1, 'hall', 'docs/memory/decision/a.md', 'h', 0, 1, 0),
                  (2, 'hall', 'docs/guide.md', 'h', 0, 1, 0);
         INSERT INTO file_content_fts (rowid, content) VALUES (1, 'quokka'), (2, 'quokka');
         PRAGMA user_version = 11;",
    )
    .expect("v11 rows");

    apply_migrations(&conn).expect("migrate to 12");

    let rows: Vec<i64> = conn
        .prepare("SELECT rowid FROM file_content_fts ORDER BY rowid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows, vec![2]);
    let files: i64 = conn
        .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
        .unwrap();
    assert_eq!(files, 2);
    assert_eq!(user_version(&conn).unwrap(), 12);
}
