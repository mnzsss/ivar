#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::infra::fs;
use crate::test_support::utf8_temp_dir;

#[test]
fn plugin_is_materialised_and_idempotent() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("plugins/ivar.js");

    let first = materialise_plugin(&path).unwrap();
    assert_eq!(first, Change::Created);
    let on_disk = fs::read_text(&path).unwrap().unwrap();
    assert!(on_disk.contains("shell.env"), "should hook shell.env");
    assert!(
        on_disk.contains("tool.execute.before"),
        "should hook tool.execute.before"
    );
    assert!(
        on_disk.contains("ivar guard --provider opencode"),
        "should call ivar guard"
    );
    assert!(
        on_disk.contains("ivar session env"),
        "should call ivar session env"
    );

    let second = materialise_plugin(&path).unwrap();
    assert_eq!(second, Change::Unchanged);
}

#[test]
fn remove_plugin_deletes_the_file() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("plugins/ivar.js");

    materialise_plugin(&path).unwrap();
    let change = remove_plugin(&path).unwrap();
    assert_eq!(change, Change::Removed);
    assert!(!path.exists());
}

#[test]
fn remove_plugin_on_absent_file_is_unchanged() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("plugins/ivar.js");

    assert_eq!(remove_plugin(&path).unwrap(), Change::Unchanged);
}
