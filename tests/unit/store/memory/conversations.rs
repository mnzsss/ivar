#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use super::*;

fn plant(db: &MemoryDb, rows: &[(&str, &str, &str, &str, u8)]) {
    for (n, (event, session, native, at, importance)) in rows.iter().enumerate() {
        db.conn()
            .execute(
                "INSERT INTO observations (event_id, source_event, occurred_at, session_id,
                    native_session_id, provider, ivar_version, importance, body)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'claude-code', '0', ?6, '{}')",
                rusqlite::params![format!("e{n}"), event, at, session, native, importance],
            )
            .unwrap();
    }
}

fn db() -> (tempfile::TempDir, MemoryDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDb::open(&dir.path().join("memory.db")).unwrap();
    (dir, db)
}

#[test]
fn heads_group_session_rows_by_native_id_and_flag_the_end() {
    let (_guard, db) = db();
    plant(
        &db,
        &[
            (
                "session.started",
                "s1",
                "n1",
                "2026-11-01T10:00:00.000000Z",
                1,
            ),
            ("session.tool", "s1", "n1", "2026-11-01T10:05:00.000000Z", 4),
            (
                "session.ended",
                "s1",
                "n1",
                "2026-11-01T10:06:00.000000Z",
                1,
            ),
            (
                "session.prompted",
                "",
                "n2",
                "2026-11-01T11:00:00.000000Z",
                5,
            ),
            ("delivery.pr", "s1", "", "2026-11-01T10:05:30.000000Z", 4),
            (
                "session.tool",
                "s0",
                "old",
                "2026-09-01T10:00:00.000000Z",
                4,
            ),
        ],
    );

    let heads = conversation_heads(&db, "2026-10-01T00:00:00.000000Z", None).unwrap();

    assert_eq!(heads.len(), 2);
    assert_eq!(heads[0].native_session_id, "n1");
    assert_eq!(heads[0].session_id, "s1");
    assert_eq!(heads[0].first_at, "2026-11-01T10:00:00.000000Z");
    assert_eq!(heads[0].last_at, "2026-11-01T10:06:00.000000Z");
    assert!(heads[0].ended);
    assert_eq!(heads[1].native_session_id, "n2");
    assert!(!heads[1].ended);
    let only_s1 = conversation_heads(&db, "2026-10-01T00:00:00.000000Z", Some("s1")).unwrap();
    assert_eq!(only_s1.len(), 1);
}

#[test]
fn rows_of_a_conversation_and_unattached_delivery_rows_are_read_in_id_order() {
    let (_guard, db) = db();
    plant(
        &db,
        &[
            (
                "session.started",
                "s1",
                "n1",
                "2026-11-01T10:00:00.000000Z",
                1,
            ),
            ("delivery.pr", "s1", "", "2026-11-01T10:05:30.000000Z", 4),
            ("session.tool", "s1", "n1", "2026-11-01T10:05:00.000000Z", 6),
            ("delivery.pushed", "", "", "2026-11-01T10:05:40.000000Z", 4),
        ],
    );

    let rows = conversation_rows(&db, "n1").unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r.source_event.as_str())
            .collect::<Vec<_>>(),
        ["session.started", "session.tool"]
    );
    assert_eq!(rows[1].importance, 6);
    let unattached = unattached_rows(&db, "s1").unwrap();
    assert_eq!(unattached.len(), 1);
    assert_eq!(unattached[0].source_event, "delivery.pr");
    assert!(unattached_rows(&db, "").unwrap().is_empty());
}
