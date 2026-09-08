//! Unit tests for `crate::action::feature::select`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::confirm;
use crate::action::feature::create::{CreateInput, create as create_action};
use crate::test_support::seeded_hall;

fn create_test_feature(ctx: &Ctx, name: &str) {
    create_action(
        ctx,
        CreateInput {
            name: name.to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
}

#[test]
fn resolve_single_feature_returns_explicit_when_provided() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let result = resolve_single_feature(&ctx, Some("explicit-feat".into()), "Select feature");
    assert_eq!(result.unwrap(), "explicit-feat");
}

#[test]
fn resolve_single_feature_fails_when_non_interactive_and_omitted() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root).with_confirm(confirm::reporter(false));
    let err = resolve_single_feature(&ctx, None, "Select feature").unwrap_err();
    assert_eq!(err.code, "feature.missing_argument");
}

#[test]
fn resolve_single_feature_fails_when_interactive_and_no_features_exist() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root).with_confirm(confirm::fixed_select_one(true, Some(0)));
    let err = resolve_single_feature(&ctx, None, "Select feature").unwrap_err();
    assert_eq!(err.code, "feature.no_features_available");
}

#[test]
fn resolve_single_feature_fails_when_interactive_and_selection_cancelled() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    create_test_feature(&ctx, "alpha");

    let ctx = ctx.with_confirm(confirm::fixed_select_one(true, None));
    let err = resolve_single_feature(&ctx, None, "Select feature").unwrap_err();
    assert_eq!(err.code, "feature.selection_cancelled");
}

#[test]
fn resolve_single_feature_prompts_and_returns_selection_when_interactive() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    create_test_feature(&ctx, "alpha");
    create_test_feature(&ctx, "beta");

    let ctx = ctx.with_confirm(confirm::fixed_select_one(true, Some(1)));
    let result = resolve_single_feature(&ctx, None, "Select feature");
    assert_eq!(result.unwrap(), "beta");
}

#[test]
fn resolve_multi_features_returns_single_explicit_when_provided() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);
    let result = resolve_multi_features(&ctx, Some("explicit-feat".into()), "Select features");
    assert_eq!(result.unwrap(), vec!["explicit-feat"]);
}

#[test]
fn resolve_multi_features_fails_when_non_interactive_and_omitted() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root).with_confirm(confirm::reporter(false));
    let err = resolve_multi_features(&ctx, None, "Select features").unwrap_err();
    assert_eq!(err.code, "feature.missing_argument");
}

#[test]
fn resolve_multi_features_fails_when_interactive_and_no_features_exist() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root).with_confirm(confirm::fixed_select(true, vec![0]));
    let err = resolve_multi_features(&ctx, None, "Select features").unwrap_err();
    assert_eq!(err.code, "feature.no_features_available");
}

#[test]
fn resolve_multi_features_fails_when_interactive_and_selection_cancelled() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    create_test_feature(&ctx, "alpha");

    let ctx = ctx.with_confirm(confirm::fixed_select(true, vec![]));
    let err = resolve_multi_features(&ctx, None, "Select features").unwrap_err();
    assert_eq!(err.code, "feature.selection_cancelled");
}

#[test]
fn resolve_multi_features_prompts_and_returns_selection_when_interactive() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    create_test_feature(&ctx, "alpha");
    create_test_feature(&ctx, "beta");

    let ctx = ctx.with_confirm(confirm::fixed_select(true, vec![0, 1]));
    let result = resolve_multi_features(&ctx, None, "Select features");
    assert_eq!(result.unwrap(), vec!["alpha", "beta"]);
}
