#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use serde::Deserialize;

use super::*;

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(default)]
struct Doc {
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    closed_at: Option<String>,
}

#[test]
fn no_frontmatter_is_normal_not_an_error() {
    let source = "just a plain body\nwith two lines\n";
    let split = split(source).unwrap();
    assert_eq!(split.frontmatter, None);
    assert_eq!(split.body, source);
}

#[test]
fn empty_input_is_normal_too() {
    let split = split("").unwrap();
    assert_eq!(split.frontmatter, None);
    assert_eq!(split.body, "");
}

#[test]
fn fence_must_be_at_the_very_start() {
    // A `---` line that isn't the first line of the document is body, not a
    // fence — nothing opened it.
    let source = "intro\n---\nkey: value\n---\nbody\n";
    let split = split(source).unwrap();
    assert_eq!(split.frontmatter, None);
    assert_eq!(split.body, source);
}

#[test]
fn a_line_with_extra_dashes_is_not_a_fence() {
    let source = "----\nnot frontmatter\n";
    let split = split(source).unwrap();
    assert_eq!(split.frontmatter, None);
    assert_eq!(split.body, source);
}

#[test]
fn splits_frontmatter_from_body() {
    let source = "---\noutcome: shipped\n---\nbody line one\nbody line two\n";
    let split = split(source).unwrap();
    assert_eq!(split.frontmatter, Some("outcome: shipped\n"));
    assert_eq!(split.body, "body line one\nbody line two\n");
}

#[test]
fn dashes_inside_the_body_are_not_a_terminator_once_closed() {
    let source = "---\noutcome: shipped\n---\nbody\n---\nmore body\n";
    let split = split(source).unwrap();
    assert_eq!(split.frontmatter, Some("outcome: shipped\n"));
    assert_eq!(split.body, "body\n---\nmore body\n");
}

#[test]
fn crlf_input_survives_with_body_unchanged() {
    let source = "---\r\noutcome: shipped\r\n---\r\nbody line\r\nsecond line\r\n";
    let split = split(source).unwrap();
    assert_eq!(split.frontmatter, Some("outcome: shipped\r\n"));
    assert_eq!(split.body, "body line\r\nsecond line\r\n");
}

#[test]
fn unterminated_opening_fence_is_a_hard_error_naming_the_line() {
    let source = "---\noutcome: shipped\nno closing fence\n";
    let error = split(source).unwrap_err();
    match error {
        FrontmatterError::UnterminatedFence { line } => assert_eq!(line, 1),
        other => panic!("expected UnterminatedFence, got {other:?}"),
    }
}

#[test]
fn unterminated_fence_with_nothing_after_it_is_also_an_error() {
    let error = split("---\n").unwrap_err();
    assert!(matches!(error, FrontmatterError::UnterminatedFence { .. }));

    let error = split("---").unwrap_err();
    assert!(matches!(error, FrontmatterError::UnterminatedFence { .. }));
}

#[test]
fn empty_frontmatter_block_is_valid_and_deserializes_as_empty_mapping() {
    let split = split("---\n---\nbody\n").unwrap();
    assert_eq!(split.frontmatter, Some(""));
    assert_eq!(split.body, "body\n");

    let doc: Doc = parse("---\n---\nbody\n").unwrap();
    assert_eq!(doc, Doc::default());
}

#[test]
fn no_frontmatter_also_deserializes_as_empty_mapping() {
    let doc: Doc = parse("no frontmatter here\n").unwrap();
    assert_eq!(doc, Doc::default());
}

#[test]
fn parse_deserializes_the_block() {
    let source = "---\noutcome: shipped\nclosed_at: '2026-08-06'\n---\nbody\n";
    let doc: Doc = parse(source).unwrap();
    assert_eq!(
        doc,
        Doc {
            outcome: Some("shipped".to_owned()),
            closed_at: Some("2026-08-06".to_owned()),
        }
    );
}

#[test]
fn parse_surfaces_invalid_yaml() {
    // An unclosed flow mapping is invalid YAML, not a Rust-level shape
    // mismatch, so this should come back through the `Invalid` variant.
    let source = "---\noutcome: [shipped\n---\nbody\n";
    let error = parse::<Doc>(source).unwrap_err();
    assert!(matches!(error, FrontmatterError::Invalid(_)));
}

#[test]
fn replace_swaps_frontmatter_and_leaves_the_body_byte_for_byte() {
    let source = "---\noutcome: pending\n---\nA human wrote this.\nDo not touch it.\n";
    let updated = Doc {
        outcome: Some("shipped".to_owned()),
        closed_at: Some("2026-08-06".to_owned()),
    };

    let result = replace(source, &updated).unwrap();

    let split_before = split(source).unwrap();
    let split_after = split(&result).unwrap();
    assert_eq!(split_after.body, split_before.body);
    assert_eq!(split_after.body, "A human wrote this.\nDo not touch it.\n");

    let round_tripped: Doc = parse(&result).unwrap();
    assert_eq!(round_tripped, updated);
}

#[test]
fn replace_on_a_document_with_no_frontmatter_prepends_a_fresh_block() {
    let source = "no frontmatter at all\njust body\n";
    let updated = Doc {
        outcome: Some("shipped".to_owned()),
        closed_at: None,
    };

    let result = replace(source, &updated).unwrap();
    let split = split(&result).unwrap();
    assert_eq!(split.body, source);

    let round_tripped: Doc = parse(&result).unwrap();
    assert_eq!(round_tripped, updated);
}

#[test]
fn replace_preserves_a_body_with_no_trailing_newline() {
    let source = "---\noutcome: pending\n---\nbody with no trailing newline";
    let updated = Doc::default();

    let result = replace(source, &updated).unwrap();
    let split = split(&result).unwrap();
    assert_eq!(split.body, "body with no trailing newline");
}

#[test]
fn replace_preserves_a_crlf_body() {
    let source = "---\r\noutcome: pending\r\n---\r\nline one\r\nline two\r\n";
    let updated = Doc {
        outcome: Some("shipped".to_owned()),
        closed_at: None,
    };

    let result = replace(source, &updated).unwrap();
    let split = split(&result).unwrap();
    assert_eq!(split.body, "line one\r\nline two\r\n");
}

#[test]
fn replace_surfaces_unterminated_fences_in_the_source() {
    let source = "---\nno closing fence\n";
    let error = replace(source, &Doc::default()).unwrap_err();
    assert!(matches!(error, FrontmatterError::UnterminatedFence { .. }));
}
