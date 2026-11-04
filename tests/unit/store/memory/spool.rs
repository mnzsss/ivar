#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
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

    assert_eq!(append(&dir, &key, "{\"n\":1}").unwrap(), Appended::Written);
    assert_eq!(append(&dir, &key, "{\"n\":2}").unwrap(), Appended::Written);

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
    assert_eq!(append(&dir, &key, &big).unwrap(), Appended::Written);

    assert_eq!(append(&dir, &key, "overflow").unwrap(), Appended::Dropped);
    assert_eq!(append(&dir, &key, "again").unwrap(), Appended::Dropped);

    assert_eq!(dropped_count(&dir), 2);
    assert!(has_pending(&dir));
    let size = std::fs::metadata(dir.join("s.jsonl")).unwrap().len();
    assert!(size <= SPOOL_CAP_BYTES);
}
