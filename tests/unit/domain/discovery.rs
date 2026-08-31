#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::name::FeatureName;

fn name() -> FeatureName {
    FeatureName::new("checkout-refactor").unwrap()
}

#[test]
fn new_starts_a_doc_in_exploring_with_the_name_as_title_fallback() {
    let doc = DiscoveryDoc::new(&name(), None, "2026-08-30T10:00:00Z");

    assert_eq!(doc.frontmatter.name, "checkout-refactor");
    assert_eq!(doc.frontmatter.title, "checkout-refactor");
    assert_eq!(doc.frontmatter.status, DiscoveryStatus::Exploring);
    assert_eq!(doc.frontmatter.created_at, "2026-08-30T10:00:00Z");
    assert_eq!(doc.frontmatter.updated_at, "2026-08-30T10:00:00Z");
    assert!(doc.frontmatter.sessions.is_empty());
    assert!(doc.is_writable());
}

#[test]
fn new_uses_an_explicit_title_when_given() {
    let doc = DiscoveryDoc::new(&name(), Some("Checkout, revisited"), "2026-08-30T10:00:00Z");
    assert_eq!(doc.frontmatter.title, "Checkout, revisited");
    assert_eq!(doc.frontmatter.name, "checkout-refactor");
}

#[test]
fn is_writable_is_false_when_status_is_unknown() {
    let doc = DiscoveryDoc {
        frontmatter: Frontmatter {
            status: DiscoveryStatus::Unknown,
            ..Frontmatter::default()
        },
        body: String::new(),
    };
    assert!(!doc.is_writable());
}
