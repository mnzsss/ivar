//! Unit tests for `crate::action::feedback::show`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::str_to_string
)]

use super::*;
use crate::domain::feedback::*;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

#[test]
fn show_redacted_matches_issue_preview_and_unknown_errors() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let ctx = Ctx::new(root.clone());

    let e = FeedbackEntry {
        id: "001-sensitive".to_string(),
        frontmatter: Frontmatter {
            title: "Sensitive bug".to_string(),
            kind: FeedbackKind::Bug,
            status: FeedbackStatus::Open,
            created_at: "2026-10-05T12:00:00Z".to_string(),
            ivar_version: "0.14.0".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            provider: None,
            session: None,
            feature: None,
            published_url: None,
            extra: Default::default(),
        },
        body: format!("Crash occurred inside {}/src/lib.rs", root),
    };
    crate::store::feedback::write(&layout, &e).unwrap();

    let res_plain = show(
        &ctx,
        ShowInput {
            id: "001-sensitive".to_string(),
            redacted: false,
        },
    )
    .unwrap();
    assert!(res_plain.value.entry.body.contains(root.as_str()));

    let res_redacted = show(
        &ctx,
        ShowInput {
            id: "001-sensitive".to_string(),
            redacted: true,
        },
    )
    .unwrap();
    let r = redactions(&layout);
    let (preview_title, preview_body) = issue_preview(&e, &r);
    assert_eq!(res_redacted.value.entry.frontmatter.title, preview_title);
    assert_eq!(res_redacted.value.entry.body, preview_body);
    assert!(!res_redacted.value.entry.body.contains(root.as_str()));
    assert!(res_redacted.value.entry.body.contains("<hall>/src/lib.rs"));

    let err = show(
        &ctx,
        ShowInput {
            id: "999-missing".to_string(),
            redacted: false,
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "feedback.not_found");
}
