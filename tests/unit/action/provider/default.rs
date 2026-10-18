#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::provider::add::{AddInput, add};
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

fn persisted_default(root: &Utf8PathBuf) -> Provider {
    Manifest::read(&Layout::at(root.clone()))
        .unwrap()
        .unwrap()
        .providers()
        .default_provider()
}

#[test]
fn default_switches_to_a_registered_provider() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    add(
        &ctx,
        &AddInput {
            name: "opencode".to_owned(),
        },
    )
    .unwrap();

    let outcome = set_default(
        &ctx,
        &DefaultInput {
            name: "opencode".to_owned(),
        },
    )
    .unwrap()
    .value;

    assert!(outcome.changed);
    assert_eq!(outcome.previous, Provider::ClaudeCode);
    assert_eq!(outcome.default, Provider::OpenCode);
    assert_eq!(persisted_default(&root), Provider::OpenCode);
}

#[test]
fn the_current_default_changes_nothing() {
    let (_guard, root) = seeded_hall();
    let before = std::fs::read_to_string(root.join("ivar.json")).unwrap();

    let outcome = set_default(
        &Ctx::new(root.clone()),
        &DefaultInput {
            name: "claude-code".to_owned(),
        },
    )
    .unwrap()
    .value;

    assert!(!outcome.changed);
    assert_eq!(
        std::fs::read_to_string(root.join("ivar.json")).unwrap(),
        before
    );
}

#[test]
fn an_unregistered_provider_is_refused_without_writing() {
    let (_guard, root) = seeded_hall();
    let before = std::fs::read_to_string(root.join("ivar.json")).unwrap();

    let failure = set_default(
        &Ctx::new(root.clone()),
        &DefaultInput {
            name: "omp".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "provider.not_available");
    assert_eq!(
        std::fs::read_to_string(root.join("ivar.json")).unwrap(),
        before
    );
}

#[test]
fn an_unknown_id_is_refused() {
    let (_guard, root) = seeded_hall();

    let failure = set_default(
        &Ctx::new(root),
        &DefaultInput {
            name: "codex".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "provider.unknown_id");
}
