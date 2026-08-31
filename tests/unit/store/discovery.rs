#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::discovery::{DiscoveryDoc, DiscoveryStatus};
use crate::domain::name::FeatureName;

fn name() -> FeatureName {
    FeatureName::new("checkout-refactor").unwrap()
}

#[test]
fn render_then_parse_is_a_round_trip() {
    let mut doc = DiscoveryDoc::new(&name(), None, "2026-08-30T10:00:00Z");
    doc.body = "# Checkout\n\nSome prose.\n".to_owned();
    doc.frontmatter
        .sessions
        .push("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c".to_owned());

    let rendered = render(&doc).unwrap();
    let reparsed = parse(&rendered);

    assert_eq!(reparsed.frontmatter, doc.frontmatter);
    assert_eq!(reparsed.body, doc.body);
}

/// D4: ivar owns the container, the agent owns the prose. A re-render must
/// not reflow, renumber, or renormalise a single byte of the body.
#[test]
fn render_leaves_the_body_byte_for_byte() {
    let source = "---\nname: checkout-refactor\ntitle: Checkout\nstatus: exploring\ncreated_at: \"2026-08-30T10:00:00Z\"\nupdated_at: \"2026-08-30T10:00:00Z\"\nsessions: []\n---\n  odd   spacing\n\n\n- a list\n\ttab indented\n";

    let doc = parse(source);
    let rendered = render(&doc).unwrap();

    let split_rendered = crate::infra::frontmatter::split(&rendered).unwrap();
    let split_source = crate::infra::frontmatter::split(source).unwrap();
    assert_eq!(split_rendered.body, split_source.body);
}

/// D5: lenient parsing preserves unknown keys on rewrite.
#[test]
fn unknown_keys_in_front_matter_are_preserved_on_render() {
    let source = "---\nname: checkout-refactor\ntitle: Checkout\nstatus: exploring\ncreated_at: \"2026-08-30T10:00:00Z\"\nupdated_at: \"2026-08-30T10:00:00Z\"\nsessions: []\nfuture_field: 42\nnested_custom:\n  foo: bar\n---\nBody.\n";

    let doc = parse(source);
    assert!(doc.frontmatter.extra.contains_key("future_field"));
    assert!(doc.frontmatter.extra.contains_key("nested_custom"));

    let rendered = render(&doc).unwrap();
    let reparsed = parse(&rendered);
    assert_eq!(reparsed.frontmatter.extra, doc.frontmatter.extra);
}

#[test]
fn unreadable_front_matter_produces_unknown_status_and_is_not_writable() {
    let source = "---\n: : : invalid yaml\n---\nSome prose.\n";
    let doc = parse(source);

    assert_eq!(doc.frontmatter.status, DiscoveryStatus::Unknown);
    assert!(!doc.is_writable());
    assert_eq!(doc.body, source);
}

#[test]
fn unknown_status_string_produces_unknown_status_and_is_not_writable() {
    let source = "---\nname: checkout\nstatus: unrecognized_status\n---\nSome prose.\n";
    let doc = parse(source);

    assert_eq!(doc.frontmatter.status, DiscoveryStatus::Unknown);
    assert!(!doc.is_writable());
    assert_eq!(doc.body, source);
}

#[test]
fn unwritable_doc_fails_to_render() {
    let source = "---\n: : : invalid yaml\n---\nSome prose.\n";
    let doc = parse(source);

    let err = render(&doc).unwrap_err();
    assert_eq!(err.code, "discovery.unwritable");
}
