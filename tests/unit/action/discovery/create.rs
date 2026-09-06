#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::hall::{self, InitInput};
use crate::domain::discovery::DiscoveryStatus;
use crate::domain::name::SessionId;
use crate::domain::session::SessionState;
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
    assert_eq!(outcome.feature_dir, layout.feature_dir(&name));
    assert_eq!(outcome.doc, layout.discovery_doc(&name));
    assert!(fs::is_file(&outcome.doc).unwrap());

    let doc = discovery::parse(&fs::read_text(&outcome.doc).unwrap().unwrap());
    assert_eq!(doc.frontmatter.name, "checkout-refactor");
    assert_eq!(doc.frontmatter.status, DiscoveryStatus::Exploring);
    assert!(doc.is_writable());
}

#[test]
fn create_inside_discovery_session_writes_to_view_dir() {
    let (_guard, root) = hall();
    let layout = Layout::at(root.clone());
    let session_id = SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    fs::ensure_dir(&view_dir).unwrap();
    let state = SessionState::new(
        crate::domain::provider::Provider::ClaudeCode,
        "2026-01-01T00:00:00.000000000Z",
    );
    state.write(&view_dir).unwrap();

    let ctx = Ctx::new(view_dir.clone());
    let outcome = create(
        &ctx,
        CreateInput {
            name: "checkout-refactor".to_owned(),
            title: None,
        },
    )
    .unwrap()
    .value;

    assert_eq!(outcome.doc, view_dir.join("discovery.md"));
    assert!(fs::is_file(&view_dir.join("discovery.md")).unwrap());
    assert!(
        !fs::exists(&layout.discovery_doc(&FeatureName::new("checkout-refactor").unwrap()))
            .unwrap()
    );
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
fn create_writes_the_doc_in_feature_dir_and_creates_no_research_dir() {
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
    assert_eq!(outcome.feature_dir, layout.feature_dir(&name));
    assert_eq!(outcome.doc, layout.discovery_doc(&name));
    assert!(fs::is_file(&outcome.doc).unwrap());
    assert!(!fs::exists(&layout.feature_dir(&name).join("research")).unwrap());
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
