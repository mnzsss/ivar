#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::observation::{Importance, Observation};
use crate::domain::sanitize::{SanitizeContext, Sanitized};
use camino::Utf8PathBuf;

fn observation(event_id: &str, source_event: &str) -> Sanitized<Observation> {
    let observation = Observation {
        event_id: event_id.to_owned(),
        source_event: source_event.to_owned(),
        occurred_at: "2026-11-04T10:00:00.000000000Z".to_owned(),
        session_id: String::new(),
        native_session_id: "n1".to_owned(),
        provider: "claude-code".to_owned(),
        ivar_version: "0.19.0".to_owned(),
        feature: String::new(),
        parent_feature: String::new(),
        repo: String::new(),
        origin: String::new(),
        importance: Importance::session_started(),
        attempt_id: String::new(),
        body: "{}".to_owned(),
    };
    let ctx = SanitizeContext::new(Utf8PathBuf::from("/hall"), Vec::new());
    Sanitized::sanitize(observation, &ctx).unwrap()
}

fn open() -> (tempfile::TempDir, MemoryDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDb::open(&dir.path().join("memory.db")).unwrap();
    (dir, db)
}

#[test]
fn inserting_the_same_event_twice_stores_it_once() {
    let (_dir, mut db) = open();
    let batch = [observation("a", "session.started")];

    assert_eq!(insert_batch(&mut db, &batch).unwrap(), 1);
    assert_eq!(insert_batch(&mut db, &batch).unwrap(), 0);
}

#[test]
fn rows_are_counted_per_source_event() {
    let (_dir, mut db) = open();
    insert_batch(
        &mut db,
        &[
            observation("a", "session.started"),
            observation("b", "session.tool"),
            observation("c", "session.tool"),
        ],
    )
    .unwrap();

    assert_eq!(
        count_by_source_event(&db).unwrap(),
        vec![
            ("session.started".to_owned(), 1),
            ("session.tool".to_owned(), 2)
        ]
    );
}
