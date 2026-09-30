#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::test_support::utf8_temp_dir;

#[test]
fn round_trips_through_the_file_creating_its_directory() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("ivar").join("update-check.json");
    let entry = CacheEntry {
        last_checked_at: 42,
        latest_version: Some("0.13.0".to_owned()),
    };

    assert!(write(&path, &entry));
    assert_eq!(read(&path), Some(entry));
}

#[test]
fn a_missing_or_corrupt_file_reads_as_none() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("update-check.json");
    assert_eq!(read(&path), None);

    std::fs::write(&path, "not json").unwrap();
    assert_eq!(read(&path), None);
}

#[test]
fn an_unwritable_location_reports_false_instead_of_failing() {
    let (_dir, root) = utf8_temp_dir();
    let blocker = root.join("file");
    std::fs::write(&blocker, "").unwrap();
    // A regular file where the directory should be: ensure_dir must fail.
    let path = blocker.join("ivar").join("update-check.json");
    assert!(!write(
        &path,
        &CacheEntry {
            last_checked_at: 1,
            latest_version: None
        }
    ));
}
