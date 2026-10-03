//! Unit tests for `crate::infra::table` — the shared table builder and writer.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

#[test]
fn table_new_and_write_produces_exact_layout_with_header_styling() {
    let mut table = new(&["REPO", "STATE"]);
    table.add_row(vec!["api", "cloned"]);
    table.add_row(vec!["webapp", "ok"]);

    // Unstyled comfy-table to_string() has trailing padding spaces on cells and no trailing newline
    assert_eq!(
        table.to_string(),
        "REPO    STATE \napi     cloned\nwebapp  ok    "
    );

    let mut out = Vec::new();
    write(&mut out, &table).unwrap();
    let rendered = String::from_utf8(out).unwrap();

    let stripped = console::strip_ansi_codes(&rendered).to_string();
    assert_eq!(stripped, "REPO    STATE\napi     cloned\nwebapp  ok\n");

    // Verify that line 0 is painted with HEADER bold style
    assert!(rendered.starts_with("\x1b[1mREPO    STATE\x1b[0m\n"));
}

#[test]
fn table_unprimed_has_disabled_content_arrangement_and_does_not_wrap() {
    let long_text = "very_long_identifier_that_should_never_be_wrapped_when_unprimed_by_default";
    let mut table = new(&["NAME", "DETAILS"]);
    table.add_row(vec!["item1", long_text]);

    let mut out = Vec::new();
    write(&mut out, &table).unwrap();
    let stripped = console::strip_ansi_codes(&String::from_utf8(out).unwrap()).to_string();
    assert!(stripped.contains(long_text));
}
