#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::discovery::create::{self, CreateInput};
use crate::action::hall::{self, InitInput};
use crate::domain::discovery::{DiscoveryDoc, DiscoveryStatus};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::test_support::hall_root;

fn hall_with_discovery() -> (tempfile::TempDir, Utf8PathBuf) {
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
    create::create(
        &ctx,
        CreateInput {
            name: "checkout-refactor".to_owned(),
            title: None,
        },
    )
    .unwrap();
    (guard, root)
}

fn read(root: &Utf8PathBuf) -> DiscoveryDoc {
    let path =
        Layout::at(root.clone()).discovery_doc(&FeatureName::new("checkout-refactor").unwrap());
    crate::store::discovery::parse(&fs::read_text(&path).unwrap().unwrap())
}

#[rstest::rstest]
#[case(DiscoveryStatus::Converted)]
#[case(DiscoveryStatus::Abandoned)]
fn close_sets_the_terminal_status(#[case] outcome: DiscoveryStatus) {
    let (_guard, root) = hall_with_discovery();
    let ctx = Ctx::new(root.clone());

    let result = close(
        &ctx,
        CloseInput {
            name: "checkout-refactor".to_owned(),
            outcome,
        },
    )
    .unwrap()
    .value;

    assert_eq!(result.status, outcome);
    assert_eq!(read(&root).frontmatter.status, outcome);
}

/// D10 and D3: an abandoned discovery is kept, not deleted. It is the
/// cheapest information a team owns.
#[test]
fn an_abandoned_discovery_is_kept_and_still_listed() {
    let (_guard, root) = hall_with_discovery();
    let ctx = Ctx::new(root.clone());

    close(
        &ctx,
        CloseInput {
            name: "checkout-refactor".to_owned(),
            outcome: DiscoveryStatus::Abandoned,
        },
    )
    .unwrap();

    let layout = Layout::at(root.clone());
    let name = FeatureName::new("checkout-refactor").unwrap();
    assert!(fs::is_file(&layout.discovery_doc(&name)).unwrap());

    let listed = crate::action::discovery::list::list(
        &ctx,
        crate::action::discovery::list::ListInput {
            status: Some(DiscoveryStatus::Abandoned),
        },
    )
    .unwrap()
    .value;
    assert_eq!(listed.discoveries.len(), 1);
}

#[test]
fn close_preserves_the_body() {
    let (_guard, root) = hall_with_discovery();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let name = FeatureName::new("checkout-refactor").unwrap();
    let path = layout.discovery_doc(&name);

    let mut doc = crate::store::discovery::parse(&fs::read_text(&path).unwrap().unwrap());
    doc.body = "# Findings\n\n  odd   spacing kept\n".to_owned();
    fs::write_text(&path, &crate::store::discovery::render(&doc).unwrap()).unwrap();

    close(
        &ctx,
        CloseInput {
            name: "checkout-refactor".to_owned(),
            outcome: DiscoveryStatus::Converted,
        },
    )
    .unwrap();

    assert_eq!(read(&root).body, "# Findings\n\n  odd   spacing kept\n");
}

#[rstest::rstest]
#[case(DiscoveryStatus::Exploring)]
#[case(DiscoveryStatus::Unknown)]
fn close_refuses_a_non_terminal_outcome(#[case] outcome: DiscoveryStatus) {
    let (_guard, root) = hall_with_discovery();
    let ctx = Ctx::new(root.clone());

    let failure = close(
        &ctx,
        CloseInput {
            name: "checkout-refactor".to_owned(),
            outcome,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "discovery.not_a_closure");
}

#[test]
fn close_fails_for_a_name_with_no_discovery() {
    let (_guard, root) = hall_with_discovery();
    let ctx = Ctx::new(root.clone());

    let failure = close(
        &ctx,
        CloseInput {
            name: "never-started".to_owned(),
            outcome: DiscoveryStatus::Abandoned,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "discovery.not_found");
}
