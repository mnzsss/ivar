#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::name::FeatureName;
use crate::domain::provider::Provider;
use crate::infra::fs;
use crate::test_support::utf8_temp_dir;

#[test]
fn absent_state_reads_as_ok_none() {
    let (_dir, root) = utf8_temp_dir();
    let view_dir = root
        .join("sessions")
        .join("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c");
    fs::ensure_dir(&view_dir).unwrap();

    assert_eq!(SessionState::read(&view_dir).unwrap(), None);
}

#[test]
fn write_then_read_round_trips() {
    let (_dir, root) = utf8_temp_dir();
    let view_dir = root
        .join("sessions")
        .join("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c");
    fs::ensure_dir(&view_dir).unwrap();
    let mut state = SessionState::new(Provider::ClaudeCode, "2026-01-01T00:00:00.000000000Z");
    state.bind(
        FeatureName::new("checkout").unwrap(),
        "2026-01-02T00:00:00.000000000Z",
    );

    state.write(&view_dir).unwrap();
    let read_back = SessionState::read(&view_dir).unwrap().unwrap();

    assert_eq!(read_back, state);
}

#[test]
fn the_file_is_written_inside_the_view_dir_with_a_version_stamp() {
    let (_dir, root) = utf8_temp_dir();
    let view_dir = root
        .join("sessions")
        .join("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c");
    fs::ensure_dir(&view_dir).unwrap();
    let state = SessionState::new(Provider::OpenCode, "2026-01-01T00:00:00.000000000Z");

    state.write(&view_dir).unwrap();

    let text = fs::read_text(&view_dir.join("state.json"))
        .unwrap()
        .unwrap();
    assert!(text.contains("\"version\": 1"), "was: {text}");
    assert!(text.contains("\"provider\": \"opencode\""), "was: {text}");
    assert!(text.contains("\"feature\": null"), "was: {text}");
}
