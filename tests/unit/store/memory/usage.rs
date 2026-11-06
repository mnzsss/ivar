#![allow(clippy::unwrap_used)]

use super::*;
use crate::store::memory::db::MemoryDb;

fn open() -> (tempfile::TempDir, MemoryDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDb::open(&dir.path().join("memory.db")).unwrap();
    (dir, db)
}

fn record(db: &MemoryDb, source: &str, session: Option<&str>) -> i64 {
    record_query(
        db.conn(),
        &QueryRecord {
            source: source.into(),
            session: session.map(str::to_owned),
            query: "teardown".into(),
            filters: "{}".into(),
            page_count: 0,
            observation_count: 0,
            duration_ms: 3,
            error: false,
        },
    )
    .unwrap()
}

fn miss(kind: MemoryMissKind, usage_id: Option<i64>) -> MemoryMiss {
    MemoryMiss {
        kind,
        session: Some("s1".into()),
        query: None,
        filters: None,
        pattern: Some("docs/memory".into()),
        usage_id,
        fallback: false,
    }
}

#[test]
fn the_last_query_of_a_session_links_one_followup() {
    let (_dir, db) = open();
    record(&db, "cli", Some("s2"));
    let id = record(&db, "hook", Some("s1"));
    let last = last_query(db.conn(), "s1").unwrap().unwrap();
    assert_eq!(
        (last.usage_id, last.query.as_deref()),
        (id, Some("teardown"))
    );
    assert!(!has_followup_for(db.conn(), id).unwrap());
    record_miss(db.conn(), &miss(MemoryMissKind::Followup, Some(id))).unwrap();
    assert!(has_followup_for(db.conn(), id).unwrap());
    assert!(last_query(db.conn(), "nobody").unwrap().is_none());
}

#[test]
fn summary_counts_queries_without_hook_rows_and_misses_by_kind() {
    let (_dir, db) = open();
    record(&db, "cli", None);
    record(&db, "mcp", Some("s1"));
    record(&db, "hook", Some("s1"));
    record_miss(db.conn(), &miss(MemoryMissKind::Empty, None)).unwrap();
    record_miss(db.conn(), &miss(MemoryMissKind::Skipped, None)).unwrap();

    let s = summary(db.conn(), 0).unwrap();

    assert_eq!((s.queries_cli, s.queries_mcp), (1, 1));
    assert_eq!((s.empty, s.followup, s.skipped), (1, 0, 1));
    assert_eq!(prune(db.conn(), i64::MAX).unwrap(), 5);
}
