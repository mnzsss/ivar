//! Unit tests for `crate::store::feedback`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::str_to_string
)]

use super::*;
use crate::domain::feedback::{FeedbackEntry, FeedbackKind, FeedbackStatus, Frontmatter};
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;
use std::collections::BTreeMap;

#[test]
fn parse_returns_unknown_status_on_invalid_frontmatter() {
    let source = "Not a frontmatter block\nJust content";
    let entry = parse("001-test", source);
    assert_eq!(entry.id, "001-test");
    assert_eq!(entry.frontmatter.status, FeedbackStatus::Unknown);
    assert_eq!(entry.body, source);
    assert!(!entry.is_writable());
}

#[test]
fn parse_returns_unknown_status_on_garbage_yaml_and_render_refuses() {
    let source = "---\n: : : invalid yaml\n---\nSome prose.\n";
    let entry = parse("001-test", source);
    assert_eq!(entry.id, "001-test");
    assert_eq!(entry.frontmatter.status, FeedbackStatus::Unknown);
    assert_eq!(entry.body, source);
    assert!(!entry.is_writable());

    let err = render(&entry).unwrap_err();
    assert_eq!(err.code, "feedback.unwritable");
}

#[test]
fn parse_and_render_roundtrip_preserves_unknown_keys_and_body_bytes() {
    let source = "---\ntitle: Crash on launch\nkind: bug\nstatus: open\ncreated_at: \"2026-10-05T12:00:00Z\"\nivar_version: 0.14.0\nos: linux\narch: x86_64\nfuture_field: 42\nnested_custom:\n  foo: bar\n---\n  odd   spacing\n\n- a list\n\ttab indented\n";
    let parsed = parse("001-bug", source);
    assert!(parsed.frontmatter.extra.contains_key("future_field"));
    assert!(parsed.frontmatter.extra.contains_key("nested_custom"));
    assert_eq!(parsed.frontmatter.status, FeedbackStatus::Open);

    let rendered = render(&parsed).unwrap();
    let reparsed = parse("001-bug", &rendered);
    assert_eq!(reparsed.frontmatter.extra, parsed.frontmatter.extra);
    assert_eq!(reparsed.body, parsed.body);
    assert_eq!(reparsed, parsed);

    let split_rendered = crate::infra::frontmatter::split(&rendered).unwrap();
    let split_source = crate::infra::frontmatter::split(source).unwrap();
    assert_eq!(split_rendered.body, split_source.body);
}

#[test]
fn parse_and_render_roundtrip() {
    let entry = FeedbackEntry {
        id: "001-bug".to_owned(),
        frontmatter: Frontmatter {
            title: "Crash on launch".to_owned(),
            kind: FeedbackKind::Bug,
            status: FeedbackStatus::Open,
            created_at: "2026-10-05T12:00:00Z".to_owned(),
            ivar_version: "0.14.0".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            provider: Some("claude-code".to_owned()),
            session: None,
            feature: None,
            published_url: None,
            extra: BTreeMap::new(),
        },
        body: "Detailed bug report description\n".to_owned(),
    };

    let rendered = render(&entry).unwrap();
    let parsed = parse("001-bug", &rendered);
    assert_eq!(parsed, entry);
}

#[test]
fn create_list_read_write_lifecycle() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);

    // Create first entry
    let e1 = create(&layout, "First Bug", |id| FeedbackEntry {
        id: id.to_owned(),
        frontmatter: Frontmatter {
            title: "First Bug".to_owned(),
            kind: FeedbackKind::Bug,
            status: FeedbackStatus::Open,
            created_at: "2026-10-05T12:00:00Z".to_owned(),
            ivar_version: "0.14.0".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            provider: None,
            session: None,
            feature: None,
            published_url: None,
            extra: BTreeMap::new(),
        },
        body: "First bug content".to_owned(),
    })
    .unwrap();

    assert_eq!(e1.id, "001-first-bug");

    // Create second entry
    let e2 = create(&layout, "Second Proposal", |id| FeedbackEntry {
        id: id.to_owned(),
        frontmatter: Frontmatter {
            title: "Second Proposal".to_owned(),
            kind: FeedbackKind::Proposal,
            status: FeedbackStatus::Open,
            created_at: "2026-10-05T12:05:00Z".to_owned(),
            ivar_version: "0.14.0".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            provider: None,
            session: None,
            feature: None,
            published_url: None,
            extra: BTreeMap::new(),
        },
        body: "Second proposal content".to_owned(),
    })
    .unwrap();

    assert_eq!(e2.id, "002-second-proposal");

    // List: newest first by seq descending
    let entries = list(&layout).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].id, "002-second-proposal");
    assert_eq!(entries[1].id, "001-first-bug");

    // Read single
    let read_e1 = read(&layout, "001-first-bug").unwrap().unwrap();
    assert_eq!(read_e1.frontmatter.title, "First Bug");

    // Read missing
    let read_missing = read(&layout, "999-missing").unwrap();
    assert!(read_missing.is_none());

    // Update via write
    let mut updated = e1;
    updated.frontmatter.status = FeedbackStatus::Published;
    updated.frontmatter.published_url = Some("https://github.com/mnzsss/ivar/issues/42".to_owned());
    write(&layout, &updated).unwrap();

    let read_updated = read(&layout, "001-first-bug").unwrap().unwrap();
    assert_eq!(read_updated.frontmatter.status, FeedbackStatus::Published);
    assert_eq!(
        read_updated.frontmatter.published_url.as_deref(),
        Some("https://github.com/mnzsss/ivar/issues/42")
    );
}

#[test]
fn pre_existing_file_races_to_next_sequence() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);

    // Pre-create 001-first-bug.md directly on disk
    let doc_path = layout.feedback_doc("001-first-bug");
    crate::infra::fs::ensure_dir(&layout.feedback_dir()).unwrap();
    crate::infra::fs::write_text(&doc_path, "pre-existing").unwrap();

    // Now create via create() with same title
    let entry = create(&layout, "First Bug", |id| FeedbackEntry {
        id: id.to_owned(),
        frontmatter: Frontmatter {
            title: "First Bug".to_owned(),
            kind: FeedbackKind::Bug,
            status: FeedbackStatus::Open,
            created_at: "2026-10-05T12:00:00Z".to_owned(),
            ivar_version: "0.14.0".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            provider: None,
            session: None,
            feature: None,
            published_url: None,
            extra: BTreeMap::new(),
        },
        body: "New bug content".to_owned(),
    })
    .unwrap();

    // It should have bumped to 002-first-bug
    assert_eq!(entry.id, "002-first-bug");
}
