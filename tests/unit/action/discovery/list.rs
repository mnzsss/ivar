#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::discovery::create::{self, CreateInput};
use crate::action::hall::{self, InitInput};
use crate::domain::discovery::DiscoveryStatus;
use crate::domain::name::SessionId;
use crate::infra::fs;
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

fn start(ctx: &Ctx, name: &str) {
    create::create(
        ctx,
        CreateInput {
            name: name.to_owned(),
            title: None,
        },
    )
    .unwrap();
}

#[test]
fn list_reports_every_discovery_by_name() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());
    start(&ctx, "checkout-refactor");
    start(&ctx, "auth-rewrite");

    let outcome = list(&ctx, ListInput { status: None }).unwrap().value;

    let names: Vec<&str> = outcome
        .discoveries
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["auth-rewrite", "checkout-refactor"],
        "sorted by name"
    );
    assert!(
        outcome
            .discoveries
            .iter()
            .all(|d| d.status == DiscoveryStatus::Exploring)
    );
}

#[test]
fn list_is_empty_in_a_hall_with_no_discoveries() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());

    let outcome = list(&ctx, ListInput { status: None }).unwrap().value;

    assert!(outcome.discoveries.is_empty());
}

/// A folder under `.ivar/features/` with no `discovery.md` is skipped.
#[test]
fn list_ignores_a_folder_without_a_discovery_doc() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let dir = layout.features_dir().join("some-folder");
    fs::ensure_dir(&dir).unwrap();
    fs::write_text(
        &dir.join("notes.md"),
        "just notes
",
    )
    .unwrap();

    let outcome = list(&ctx, ListInput { status: None }).unwrap().value;

    assert!(outcome.discoveries.is_empty());
}

/// D5: an unreadable header is reported, never hidden and never repaired.
#[test]
fn list_reports_an_unreadable_doc_as_unknown() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let dir = layout.features_dir().join("broken-doc");
    fs::ensure_dir(&dir).unwrap();
    fs::write_text(
        &dir.join("discovery.md"),
        "no front matter at all
",
    )
    .unwrap();

    let outcome = list(&ctx, ListInput { status: None }).unwrap().value;

    assert_eq!(outcome.discoveries.len(), 1);
    assert_eq!(outcome.discoveries[0].status, DiscoveryStatus::Unknown);
}

#[test]
fn list_scans_both_features_and_unconverted_sessions() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());

    // 1. Converted feature with discovery doc at .ivar/features/auth-rewrite/discovery.md
    let feature_name = FeatureName::new("auth-rewrite").unwrap();
    let feature_dir = layout.feature_dir(&feature_name);
    fs::ensure_dir(&feature_dir).unwrap();
    let feature_doc = layout.discovery_doc(&feature_name);
    fs::write_text(
        &feature_doc,
        "---
name: auth-rewrite
title: Auth Rewrite
status: exploring
sessions: []
updated_at: 2026-01-01T00:00:00.000000000Z
---
# Auth
",
    )
    .unwrap();

    // 2. Unconverted discovery session with doc at .ivar/sessions/<id>/discovery.md
    let session_id = SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap();
    let session_dir = layout.discovery_session(&session_id);
    fs::ensure_dir(&session_dir).unwrap();
    let session_doc = session_dir.join("discovery.md");
    fs::write_text(
        &session_doc,
        "---
name: checkout-poc
title: Checkout POC
status: exploring
sessions:
  - 2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c
updated_at: 2026-01-01T00:00:00.000000000Z
---
# Checkout POC
",
    )
    .unwrap();

    let outcome = list(&ctx, ListInput { status: None }).unwrap().value;

    assert_eq!(outcome.discoveries.len(), 2);
    assert_eq!(
        outcome.discoveries[0].name.as_str(),
        "2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c"
    );
    assert_eq!(outcome.discoveries[0].title, "Checkout POC");
    assert_eq!(outcome.discoveries[0].status, DiscoveryStatus::Exploring);
    assert_eq!(outcome.discoveries[1].name.as_str(), "auth-rewrite");
    assert_eq!(outcome.discoveries[1].title, "Auth Rewrite");
    assert_eq!(outcome.discoveries[1].status, DiscoveryStatus::Exploring);
}

#[test]
fn list_filters_by_status() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());
    start(&ctx, "checkout-refactor");

    let matching = list(
        &ctx,
        ListInput {
            status: Some(DiscoveryStatus::Exploring),
        },
    )
    .unwrap()
    .value;
    assert_eq!(matching.discoveries.len(), 1);

    let other = list(
        &ctx,
        ListInput {
            status: Some(DiscoveryStatus::Abandoned),
        },
    )
    .unwrap()
    .value;
    assert!(other.discoveries.is_empty());
}

#[test]
fn show_prints_the_doc_and_can_print_only_its_path() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());
    start(&ctx, "checkout-refactor");

    let outcome = super::super::show::show(
        &ctx,
        super::super::show::ShowInput {
            name: "checkout-refactor".to_owned(),
            path_only: false,
        },
    )
    .unwrap()
    .value;

    let layout = Layout::at(root.clone());
    let name = FeatureName::new("checkout-refactor").unwrap();
    assert_eq!(outcome.path, layout.discovery_doc(&name));
    let content = outcome.content.unwrap();
    assert!(
        content.starts_with(
            "---
name: checkout-refactor
"
        ),
        "content: {content}"
    );

    let path_only = super::super::show::show(
        &ctx,
        super::super::show::ShowInput {
            name: "checkout-refactor".to_owned(),
            path_only: true,
        },
    )
    .unwrap()
    .value;

    assert_eq!(path_only.path, layout.discovery_doc(&name));
    assert!(path_only.content.is_none());
}
