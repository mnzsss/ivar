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

/// D11: `docs/` also holds the hall's own flat topic documentation. `list`
/// must not report `docs/updates/` as a unit of work, and no ivar command
/// may touch it.
#[test]
fn list_ignores_the_halls_own_topic_directories() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());
    start(&ctx, "checkout-refactor");

    let layout = Layout::at(root.clone());
    for topic in ["product", "updates", "repo-relations"] {
        let dir = layout.work_docs_root().join(topic);
        fs::ensure_dir(&dir).unwrap();
        fs::write_text(&dir.join("001-something.md"), "# A topic\n").unwrap();
    }
    // Even a folder carrying a discovery.md is skipped when its name is not
    // a valid work name — the name is the gate, not the file.
    let odd = layout.work_docs_root().join("Not A Work Name");
    fs::ensure_dir(&odd).unwrap();
    fs::write_text(&odd.join("discovery.md"), "---\nname: x\n---\n").unwrap();

    let outcome = list(&ctx, ListInput { status: None }).unwrap().value;

    let names: Vec<&str> = outcome
        .discoveries
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    assert_eq!(names, vec!["checkout-refactor"]);
}

/// A folder under `docs/` with no `discovery.md` is the team's, not ivar's.
#[test]
fn list_ignores_a_folder_without_a_discovery_doc() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let dir = layout.work_docs_root().join("some-folder");
    fs::ensure_dir(&dir).unwrap();
    fs::write_text(&dir.join("notes.md"), "just notes\n").unwrap();

    let outcome = list(&ctx, ListInput { status: None }).unwrap().value;

    assert!(outcome.discoveries.is_empty());
}

/// D5: an unreadable header is reported, never hidden and never repaired.
#[test]
fn list_reports_an_unreadable_doc_as_unknown() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let dir = layout.work_docs_root().join("broken-doc");
    fs::ensure_dir(&dir).unwrap();
    fs::write_text(&dir.join("discovery.md"), "no front matter at all\n").unwrap();

    let outcome = list(&ctx, ListInput { status: None }).unwrap().value;

    assert_eq!(outcome.discoveries.len(), 1);
    assert_eq!(outcome.discoveries[0].status, DiscoveryStatus::Unknown);
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

    assert!(outcome.content.is_some());
    assert!(
        outcome
            .path
            .as_str()
            .ends_with("docs/checkout-refactor/discovery.md")
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

    assert!(path_only.content.is_none());
}

#[test]
fn show_fails_for_a_name_with_no_discovery() {
    let (_guard, root) = hall();
    let ctx = Ctx::new(root.clone());

    let failure = super::super::show::show(
        &ctx,
        super::super::show::ShowInput {
            name: "never-started".to_owned(),
            path_only: false,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "discovery.not_found");
}
