#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

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

fn scoped(event_id: &str, session_id: &str, feature: &str, at: &str) -> Sanitized<Observation> {
    let mut o = observation(event_id, "session.tool").into_observation();
    o.session_id = session_id.to_owned();
    o.feature = feature.to_owned();
    o.occurred_at = at.to_owned();
    o.importance = Importance::session_tool(crate::domain::observation::ToolEffect::Failed);
    let ctx = SanitizeContext::new(Utf8PathBuf::from("/hall"), Vec::new());
    Sanitized::sanitize(o, &ctx).unwrap()
}

#[test]
fn feature_observations_include_the_feature_its_descendants_and_its_sessions() {
    let (_dir, mut db) = open();
    insert_batch(
        &mut db,
        &[
            scoped("discovery", "s1", "", "2026-11-05T10:00:00Z"),
            scoped("feature", "s1", "login", "2026-11-05T11:00:00Z"),
            scoped("child", "s2", "login-api", "2026-11-05T12:00:00Z"),
            scoped("other-session", "s3", "", "2026-11-05T09:00:00Z"),
            scoped("other-feature", "", "billing", "2026-11-05T08:00:00Z"),
            scoped("anonymous", "", "", "2026-11-05T07:00:00Z"),
        ],
    )
    .unwrap();

    let rows = feature_observations(&db, &["login".to_owned(), "login-api".to_owned()]).unwrap();

    let ids: Vec<&str> = rows.iter().map(|r| r.session_id.as_str()).collect();
    assert_eq!(ids, ["s1", "s1", "s2"]);
    assert_eq!(rows[0].feature, "");
    assert_eq!(rows[1].feature, "login");
    assert_eq!(rows[0].importance, 6);
}

#[test]
fn feature_observations_of_no_feature_is_empty() {
    let (_dir, mut db) = open();
    insert_batch(
        &mut db,
        &[scoped("a", "s1", "login", "2026-11-05T10:00:00Z")],
    )
    .unwrap();
    assert!(feature_observations(&db, &[]).unwrap().is_empty());
}

#[test]
fn a_session_has_activity_only_with_a_prompt_or_a_tool() {
    let (_dir, mut db) = open();
    insert_batch(&mut db, &[observation("e1", "session.started")]).unwrap();
    assert!(!session_has_activity(db.conn(), "n1").unwrap());

    insert_batch(&mut db, &[observation("e2", "session.tool")]).unwrap();
    assert!(session_has_activity(db.conn(), "n1").unwrap());
    assert!(!session_has_activity(db.conn(), "other").unwrap());
}
