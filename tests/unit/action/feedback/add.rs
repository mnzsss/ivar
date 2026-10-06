//! Unit tests for `crate::action::feedback::add`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::str_to_string
)]

use super::*;
use crate::domain::feedback::FeedbackKind;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

#[test]
fn add_creates_feedback_entry_with_context_capture() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let input = AddInput {
        title: "Test feedback title".to_string(),
        kind: FeedbackKind::Bug,
        body: Some("Steps to reproduce...".to_string()),
    };

    let report = add(&ctx, input).unwrap();
    let view = report.value;
    assert_eq!(view.entry.id, "001-test-feedback-title");
    assert_eq!(view.entry.frontmatter.title, "Test feedback title");
    assert_eq!(view.entry.frontmatter.kind, FeedbackKind::Bug);
    assert_eq!(view.entry.frontmatter.status, FeedbackStatus::Open);
    assert!(!view.entry.frontmatter.ivar_version.is_empty());
    assert!(!view.entry.frontmatter.os.is_empty());
    assert!(!view.entry.frontmatter.arch.is_empty());
    // Outside of a session, provider/session/feature are None
    assert!(view.entry.frontmatter.provider.is_none());
    assert!(view.entry.frontmatter.session.is_none());
    assert!(view.entry.frontmatter.feature.is_none());

    let layout = Layout::at(root);
    assert!(crate::infra::fs::exists(&layout.feedback_doc("001-test-feedback-title")).unwrap());
}
