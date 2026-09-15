#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use camino::Utf8PathBuf;

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput};
use crate::action::hall::{self, InitInput};
use crate::domain::name::{BranchName, HallName};
use crate::domain::provider::Provider;
use crate::store::manifest::{Manifest, Providers, Repo};
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
    let layout = Layout::at(ctx.cwd.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            "https://example.com/api.git",
            BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
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
fn resolving_twice_keeps_the_first_resolution_time() {
    let (_guard, ctx) = hall_with_checkout();
    add(&ctx, add_input("checkout", "3-5")).unwrap();
    let layout = discover_hall(&ctx).unwrap();
    let name = FeatureName::new("checkout").unwrap();
    let mut stored = ReviewComments::read(&layout, &name).unwrap();
    let comment = stored.comments.get_mut(0).unwrap();
    comment.status = CommentStatus::Resolved;
    comment.resolved_at = Some(42);
    stored.write(&layout, &name).unwrap();

    let resolved = resolve(
        &ctx,
        ResolveInput {
            feature: "checkout".to_owned(),
            id: "c1".to_owned(),
        },
    )
    .unwrap()
    .value;
    assert_eq!(resolved.resolved_at, Some(42));
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

#[test]
fn rejects_a_repo_not_declared_in_the_hall() {
    let (_guard, ctx) = hall_with_checkout();
    let input = AddInput {
        repo: "web".to_owned(),
        ..add_input("checkout", "1")
    };
    assert_eq!(add(&ctx, input).unwrap_err().code, "review.unknown_repo");
}

#[test]
fn rejects_empty_absolute_or_traversing_files() {
    let (_guard, ctx) = hall_with_checkout();
    for file in ["", "/etc/passwd", "../other/lib.rs", "src/../../x"] {
        let input = AddInput {
            file: file.to_owned(),
            ..add_input("checkout", "1")
        };
        assert_eq!(
            add(&ctx, input).unwrap_err().code,
            "review.invalid_file",
            "{file}"
        );
    }
}

#[test]
fn list_rejects_an_invalid_repo_name() {
    let (_guard, ctx) = hall_with_checkout();
    let listed = list(
        &ctx,
        ListInput {
            feature: "checkout".to_owned(),
            repo: Some("../api".to_owned()),
            status: None,
        },
    );
    assert!(listed.is_err());
}

#[test]
fn parse_lines_accepts_single_and_ranges_only() {
    for (raw, expected) in [
        ("3", Some((3, 3))),
        ("3-5", Some((3, 5))),
        ("0", None),
        ("5-3", None),
        ("a", None),
        ("3-", None),
    ] {
        assert_eq!(parse_lines(raw).ok(), expected, "{raw}");
    }
}
