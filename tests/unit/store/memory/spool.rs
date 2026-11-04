#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::observation::{Importance, Observation};
use crate::domain::sanitize::SanitizeContext;
use camino::Utf8PathBuf;

fn dir() -> (tempfile::TempDir, Utf8PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let path = Utf8PathBuf::from_path_buf(tmp.path().join("spool")).unwrap();
    (tmp, path)
}

#[test]
fn appended_lines_land_in_the_session_file_in_order() {
    let (_tmp, dir) = dir();
    let key = SpoolKey::new(Some("abc-123"), "native-1");

    assert_eq!(
        append_line(&dir, &key, "{\"n\":1}").unwrap(),
        Appended::Written
    );
    assert_eq!(
        append_line(&dir, &key, "{\"n\":2}").unwrap(),
        Appended::Written
    );

    let body = std::fs::read_to_string(dir.join("abc-123.jsonl")).unwrap();
    assert_eq!(body, "{\"n\":1}\n{\"n\":2}\n");
}

#[test]
fn a_session_outside_ivar_is_keyed_by_the_native_id() {
    assert_eq!(SpoolKey::new(None, "native-1").as_str(), "native-1");
}

#[test]
fn a_key_that_could_escape_the_directory_is_hashed() {
    let key = SpoolKey::new(None, "../../etc/passwd");
    assert!(!key.as_str().contains('/') && !key.as_str().contains(".."));
}

#[test]
fn a_line_that_would_pass_the_cap_is_dropped_and_counted() {
    let (_tmp, dir) = dir();
    let key = SpoolKey::new(Some("s"), "n");
    let big = "x".repeat(usize::try_from(SPOOL_CAP_BYTES).unwrap() - 1);
    assert_eq!(append_line(&dir, &key, &big).unwrap(), Appended::Written);

    assert_eq!(
        append_line(&dir, &key, "overflow").unwrap(),
        Appended::Dropped
    );
    assert_eq!(append_line(&dir, &key, "again").unwrap(), Appended::Dropped);

    assert_eq!(dropped_count(&dir), 2);
    assert!(has_pending(&dir));
    let size = std::fs::metadata(dir.join("s.jsonl")).unwrap().len();
    assert!(size <= SPOOL_CAP_BYTES);
}

#[test]
fn append_writes_the_sanitized_observation_as_one_json_line() {
    let (_tmp, dir) = dir();
    let key = SpoolKey::new(Some("s"), "n");
    let observation = Observation {
        event_id: "e1".to_owned(),
        source_event: "session.prompted".to_owned(),
        occurred_at: "2026-11-04T10:00:00.000000000Z".to_owned(),
        session_id: "s".to_owned(),
        native_session_id: "n".to_owned(),
        provider: "claude-code".to_owned(),
        ivar_version: "0.19.0".to_owned(),
        feature: String::new(),
        parent_feature: String::new(),
        repo: String::new(),
        origin: "user".to_owned(),
        importance: Importance::session_ended(),
        attempt_id: String::new(),
        body: "token ghp_0123456789abcdefghijklmnopqrstuvwxyzAB".to_owned(),
    };
    let ctx = SanitizeContext::new(Utf8PathBuf::from("/hall"), Vec::new());
    let sanitized = Sanitized::sanitize(observation, &ctx).unwrap();

    assert_eq!(append(&dir, &key, &sanitized).unwrap(), Appended::Written);

    let text = std::fs::read_to_string(dir.join("s.jsonl")).unwrap();
    let parsed: Observation = serde_json::from_str(text.trim_end()).unwrap();
    assert_eq!(&parsed, sanitized.as_observation());
    assert!(!text.contains("ghp_"));
}
