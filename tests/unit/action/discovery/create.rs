#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::hall::{self, InitInput};
use crate::domain::discovery::DiscoveryStatus;
use crate::infra::fs;
use crate::store::discovery;
use crate::store::layout::Layout;
use crate::test_support::hall_root;

fn hall() -> (tempfile::TempDir, Utf8PathBuf) {
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
    (guard, root)
}

#[test]
fn create_writes_the_doc_and_reports_its_path() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());

    let outcome = create(
        &ctx,
        CreateInput {
            name: "checkout-refactor".to_owned(),
            title: None,
        },
    )
    .unwrap()
    .value;

    let layout = Layout::at(root.clone());
    let name = FeatureName::new("checkout-refactor").unwrap();
    assert_eq!(outcome.doc, layout.discovery_doc(&name));
    assert!(fs::is_file(&outcome.doc).unwrap());

    let doc = discovery::parse(&fs::read_text(&outcome.doc).unwrap().unwrap());
    assert_eq!(doc.frontmatter.name, "checkout-refactor");
    assert_eq!(doc.frontmatter.status, DiscoveryStatus::Exploring);
    assert!(doc.is_writable());
}

/// D3: memory may exist before execution. This is the normal order —
/// discovery first, feature later — so a missing feature is not an error.
#[test]
fn create_does_not_require_the_feature_to_exist() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let name = FeatureName::new("checkout-refactor").unwrap();

    assert!(!fs::is_dir(&layout.feature_dir(&name)).unwrap());

    let outcome = create(
        &ctx,
        CreateInput {
            name: "checkout-refactor".to_owned(),
            title: None,
        },
    )
    .unwrap()
    .value;

    assert!(fs::is_file(&outcome.doc).unwrap());
}

#[test]
fn create_also_makes_the_research_dir() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());

    create(
        &ctx,
        CreateInput {
            name: "checkout-refactor".to_owned(),
            title: None,
        },
    )
    .unwrap();

    let layout = Layout::at(root.clone());
    let name = FeatureName::new("checkout-refactor").unwrap();
    assert!(fs::is_dir(&layout.research_dir(&name)).unwrap());
}

#[test]
fn create_records_an_explicit_title() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());

    let outcome = create(
        &ctx,
        CreateInput {
            name: "checkout-refactor".to_owned(),
            title: Some("Checkout, revisited".to_owned()),
        },
    )
    .unwrap()
    .value;

    let doc = discovery::parse(&fs::read_text(&outcome.doc).unwrap().unwrap());
    assert_eq!(doc.frontmatter.title, "Checkout, revisited");
}

#[test]
fn create_refuses_to_overwrite_an_existing_doc() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());
    let input = || CreateInput {
        name: "checkout-refactor".to_owned(),
        title: None,
    };

    create(&ctx, input()).unwrap();
    let failure = create(&ctx, input()).unwrap_err();

    assert_eq!(failure.code, "discovery.already_exists");
}

/// Task 1's reserved names, enforced end to end: `docs/updates/` is the
/// hall's own topic directory, not a unit of work.
#[test]
fn create_refuses_a_reserved_name() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());

    let failure = create(
        &ctx,
        CreateInput {
            name: "updates".to_owned(),
            title: None,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "feature.reserved_name");
}
