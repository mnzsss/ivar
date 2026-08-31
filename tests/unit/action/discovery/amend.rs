#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::discovery::create::{self, CreateInput};
use crate::action::hall::{self, InitInput};
use crate::domain::discovery::DiscoveryDoc;
use crate::domain::name::FeatureName;
use crate::infra::{fs, hash};
use crate::store::discovery;
use crate::store::layout::Layout;
use crate::test_support::hall_root;

const SESSION: &str = "2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c";

fn hall_with_discovery() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();
    create::create(
        &ctx,
        CreateInput {
            name: "checkout-refactor".to_owned(),
            title: None,
        },
    )
    .unwrap();
    (guard, root)
}

fn doc_path(root: &Utf8PathBuf) -> Utf8PathBuf {
    Layout::at(root.clone()).discovery_doc(&FeatureName::new("checkout-refactor").unwrap())
}

fn read(root: &Utf8PathBuf) -> DiscoveryDoc {
    discovery::parse(&fs::read_text(&doc_path(root)).unwrap().unwrap())
}

fn append(ctx: &Ctx, content: &str) -> Result<AmendOutcome, crate::error::Failure> {
    amend(
        ctx,
        AmendInput {
            name: "checkout-refactor".to_owned(),
            content: content.to_owned(),
            merge: false,
            expected_hash: None,
            session_id: Some(SESSION.to_owned()),
        },
    )
    .map(|report| report.value)
}

#[test]
fn append_adds_a_dated_block_carrying_the_session_id() {
    let (_guard, root) = hall_with_discovery();
    let ctx = Ctx::new(root.clone());

    append(&ctx, "First finding.").unwrap();

    let doc = read(&root);
    assert!(
        doc.body.contains("## Amendment ("),
        "a dated heading: {}",
        doc.body
    );
    assert!(doc.body.contains(&format!("Session: {SESSION}")));
    assert!(doc.body.contains("First finding."));
    assert_eq!(doc.frontmatter.sessions, vec![SESSION.to_owned()]);
}

#[test]
fn a_second_append_keeps_the_first() {
    let (_guard, root) = hall_with_discovery();
    let ctx = Ctx::new(root.clone());

    append(&ctx, "First finding.").unwrap();
    append(&ctx, "Second finding.").unwrap();

    let doc = read(&root);
    assert!(
        doc.body.contains("First finding."),
        "append must never destroy"
    );
    assert!(doc.body.contains("Second finding."));
    assert_eq!(doc.body.matches("## Amendment (").count(), 2);
}

/// The same session contributing twice is recorded once — `sessions` is the
/// set of contributors, not a write log.
#[test]
fn the_same_session_is_recorded_once() {
    let (_guard, root) = hall_with_discovery();
    let ctx = Ctx::new(root.clone());

    append(&ctx, "First.").unwrap();
    append(&ctx, "Second.").unwrap();

    assert_eq!(read(&root).frontmatter.sessions, vec![SESSION.to_owned()]);
}

#[test]
fn merge_requires_an_expected_hash() {
    let (_guard, root) = hall_with_discovery();
    let ctx = Ctx::new(root.clone());

    let failure = amend(
        &ctx,
        AmendInput {
            name: "checkout-refactor".to_owned(),
            content: "Rewritten.".to_owned(),
            merge: true,
            expected_hash: None,
            session_id: Some(SESSION.to_owned()),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "discovery.merge_needs_hash");
}

#[test]
fn merge_replaces_the_body_when_the_hash_matches() {
    let (_guard, root) = hall_with_discovery();
    let ctx = Ctx::new(root.clone());
    append(&ctx, "Old thinking.").unwrap();

    let current = hash::file(&doc_path(&root)).unwrap();
    amend(
        &ctx,
        AmendInput {
            name: "checkout-refactor".to_owned(),
            content: "Rewritten from scratch.".to_owned(),
            merge: true,
            expected_hash: Some(current),
            session_id: Some(SESSION.to_owned()),
        },
    )
    .unwrap();

    let doc = read(&root);
    assert!(doc.body.contains("Rewritten from scratch."));
    assert!(
        !doc.body.contains("Old thinking."),
        "merge replaces the body"
    );
}

/// The drift guard: the caller states the version it read, and someone
/// else wrote in between.
#[test]
fn merge_refuses_a_stale_hash() {
    let (_guard, root) = hall_with_discovery();
    let ctx = Ctx::new(root.clone());
    let stale = hash::file(&doc_path(&root)).unwrap();
    append(&ctx, "Someone else wrote this.").unwrap();

    let failure = amend(
        &ctx,
        AmendInput {
            name: "checkout-refactor".to_owned(),
            content: "Rewritten.".to_owned(),
            merge: true,
            expected_hash: Some(stale),
            session_id: Some(SESSION.to_owned()),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "discovery.drift");
    assert!(
        read(&root).body.contains("Someone else wrote this."),
        "a refused merge must not have written"
    );
}

#[test]
fn amend_fails_for_a_name_with_no_discovery() {
    let (_guard, root) = hall_with_discovery();
    let ctx = Ctx::new(root.clone());

    let failure = amend(
        &ctx,
        AmendInput {
            name: "never-started".to_owned(),
            content: "x".to_owned(),
            merge: false,
            expected_hash: None,
            session_id: None,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "discovery.not_found");
}

/// D5: ivar never rewrites a header it could not read.
#[test]
fn amend_refuses_a_doc_with_unreadable_frontmatter() {
    let (_guard, root) = hall_with_discovery();
    let ctx = Ctx::new(root.clone());
    fs::write_text(&doc_path(&root), "no front matter at all\n").unwrap();

    let failure = append(&ctx, "x").unwrap_err();

    assert_eq!(failure.code, "discovery.unreadable_frontmatter");
}
