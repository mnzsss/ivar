//! Unit tests for `crate::action::execute::inbox`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::test_support::hall_root;

/// A layout and feature name to address an inbox with. No hall structure is
/// needed: `append` creates the directory it writes into.
fn fixture() -> (tempfile::TempDir, Layout, FeatureName) {
    let (guard, root) = hall_root();
    (
        guard,
        Layout::at(root),
        FeatureName::new("checkout").unwrap(),
    )
}

#[test]
fn an_inbox_that_was_never_written_reads_empty() {
    let (_guard, layout, feature) = fixture();
    assert!(read(&layout, &feature, "ws-a").unwrap().is_empty());
}

#[test]
fn replies_read_back_in_the_order_they_were_appended() {
    let (_guard, layout, feature) = fixture();

    append(&layout, &feature, "ws-a", "first answer").unwrap();
    append(&layout, &feature, "ws-a", "second answer").unwrap();
    append(&layout, &feature, "ws-b", "not for ws-a").unwrap();

    assert_eq!(
        read(&layout, &feature, "ws-a").unwrap(),
        vec!["first answer".to_owned(), "second answer".to_owned()]
    );
    assert_eq!(
        read(&layout, &feature, "ws-b").unwrap(),
        vec!["not for ws-a".to_owned()]
    );
}

/// The file is append-only, so a damaged line is permanent. It must cost the
/// line it damaged, not every reply after it.
#[test]
fn a_damaged_line_does_not_hide_the_replies_around_it() {
    let (_guard, layout, feature) = fixture();

    append(&layout, &feature, "ws-a", "before").unwrap();
    let path = layout.execution_inbox(&feature, "ws-a");
    let text = fs::read_text(&path).unwrap().unwrap();
    fs::write_text(&path, &format!("{text}{{ not json\n")).unwrap();
    append(&layout, &feature, "ws-a", "after").unwrap();

    assert_eq!(
        read(&layout, &feature, "ws-a").unwrap(),
        vec!["before".to_owned(), "after".to_owned()]
    );
}
