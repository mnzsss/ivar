#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use camino::Utf8PathBuf;

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput};
use crate::action::hall::{self, InitInput};
use crate::test_support::hall_root;

fn hall_with_checkout() -> (tempfile::TempDir, Ctx) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root);
    hall::init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();
    feature_create::create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    (guard, ctx)
}

fn add_input(feature: &str, lines: &str) -> AddInput {
    AddInput {
        feature: feature.to_owned(),
        repo: "api".to_owned(),
        file: "src/lib.rs".to_owned(),
        lines: lines.to_owned(),
        body: "rename".to_owned(),
    }
}

fn open_comments(ctx: &Ctx) -> Vec<ReviewComment> {
    list(
        ctx,
        ListInput {
            feature: "checkout".to_owned(),
            repo: None,
            status: Some(CommentStatus::Open),
        },
    )
    .unwrap()
    .value
    .comments
}

#[test]
fn add_list_and_resolve_a_range_comment() {
    let (_guard, ctx) = hall_with_checkout();
    let added = add(&ctx, add_input("checkout", "3-5")).unwrap().value;
    assert_eq!(
        (added.id.as_str(), added.line_start, added.line_end),
        ("c1", 3, 5)
    );
    assert_eq!(open_comments(&ctx).len(), 1);

    let resolved = resolve(
        &ctx,
        ResolveInput {
            feature: "checkout".to_owned(),
            id: "c1".to_owned(),
        },
    )
    .unwrap()
    .value;
    assert_eq!(resolved.status, CommentStatus::Resolved);
    assert!(resolved.resolved_at.is_some());
    assert!(open_comments(&ctx).is_empty());
}

#[test]
fn rejects_unknown_feature_bad_lines_and_unknown_id() {
    let (_guard, ctx) = hall_with_checkout();
    let bad_feature = add(&ctx, add_input("nope", "1"));
    assert_eq!(bad_feature.unwrap_err().code, "feature.not_found");
    let bad_lines = add(&ctx, add_input("checkout", "5-3"));
    assert_eq!(bad_lines.unwrap_err().code, "review.invalid_lines");
    let bad_id = resolve(
        &ctx,
        ResolveInput {
            feature: "checkout".to_owned(),
            id: "c9".to_owned(),
        },
    );
    assert_eq!(bad_id.unwrap_err().code, "review.comment_not_found");
}
