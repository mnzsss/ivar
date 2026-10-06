//! Unit tests for `crate::action::feedback::list`.
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
fn list_filters_by_status() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let ctx = Ctx::new(root);

    let e1 = FeedbackEntry {
        id: "001-open".to_string(),
        frontmatter: Frontmatter {
            title: "Open bug".to_string(),
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
        body: "body".to_string(),
    };
    crate::store::feedback::write(&layout, &e1).unwrap();

    let e2 = FeedbackEntry {
        id: "002-published".to_string(),
        frontmatter: Frontmatter {
            title: "Published proposal".to_string(),
            kind: FeedbackKind::Proposal,
            status: FeedbackStatus::Published,
            created_at: "2026-10-05T12:01:00Z".to_string(),
            ivar_version: "0.14.0".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            provider: None,
            session: None,
            feature: None,
            published_url: Some("https://github.com/mnzsss/ivar/issues/1".to_string()),
            extra: Default::default(),
        },
        body: "body".to_string(),
    };
    crate::store::feedback::write(&layout, &e2).unwrap();

    let all = list(&ctx, ListInput { status: None }).unwrap();
    assert_eq!(all.value.entries.len(), 2);

    let only_open = list(
        &ctx,
        ListInput {
            status: Some(FeedbackStatus::Open),
        },
    )
    .unwrap();
    assert_eq!(only_open.value.entries.len(), 1);
    assert_eq!(only_open.value.entries[0].id, "001-open");

    let only_published = list(
        &ctx,
        ListInput {
            status: Some(FeedbackStatus::Published),
        },
    )
    .unwrap();
    assert_eq!(only_published.value.entries.len(), 1);
    assert_eq!(only_published.value.entries[0].id, "002-published");
}
