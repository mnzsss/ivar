use camino::Utf8PathBuf;

use crate::action::Ctx;
use crate::action::confirm::fixed;
use crate::action::feature::create::{CreateInput, create};
use crate::action::feature::rename::{RenameInput, rename};
use crate::action::graph::{CleanInput, IndexInput, clean_cmd, index_cmd};
use crate::action::repo::remove::{RemoveInput, remove};
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::store::memory::db::MemoryDb;
use crate::test_support::{seeded_hall, seeded_repo};

/// Opens both databases with a graph row for `repo`, and returns the
/// memory database's bytes: the sentinel every operation must leave alone.
fn seed_both(layout: &Layout, repo: &str) -> Vec<u8> {
    let graph = GraphDb::open(layout.graph_db().as_std_path()).unwrap();
    graph.insert_repo(repo, "/x", "main", None).unwrap();
    drop(graph);
    drop(MemoryDb::open(layout.memory_db().as_std_path()).unwrap());
    std::fs::read(layout.memory_db().as_std_path()).unwrap()
}

fn assert_untouched(layout: &Layout, sentinel: &[u8]) {
    assert_eq!(
        std::fs::read(layout.memory_db().as_std_path()).unwrap(),
        sentinel,
        "memory.db changed"
    );
}

#[test]
fn graph_clean_all_never_touches_the_memory_database() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let sentinel = seed_both(&layout, "api");

    let report = clean_cmd(
        &Ctx::new(root),
        CleanInput {
            repo: None,
            feature: None,
            all: true,
            path: None,
        },
    )
    .unwrap();

    assert_eq!(report.value.repos_removed, 1);
    assert_untouched(&layout, &sentinel);
}

#[test]
fn graph_index_reconcile_never_touches_the_memory_database() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let sentinel = seed_both(&layout, "gone");

    index_cmd(
        &Ctx::new(root),
        IndexInput {
            repo: None,
            full: false,
            register_mcp: false,
            paths: Vec::new(),
        },
    )
    .unwrap();

    assert!(
        GraphDb::open_read_only(layout.graph_db().as_std_path())
            .unwrap()
            .get_repo("gone")
            .unwrap()
            .is_none()
    );
    assert_untouched(&layout, &sentinel);
}

#[test]
fn hall_cleanup_never_touches_the_memory_database() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    crate::infra::fs::ensure_dir(&root.join(".ivar/repos/stale")).unwrap();
    let sentinel = seed_both(&layout, "stale");

    let report = crate::action::hall::cleanup(&Ctx::new(root).with_confirm(fixed(true))).unwrap();

    assert_eq!(report.value.graph_forgotten, vec!["stale"]);
    assert_untouched(&layout, &sentinel);
}

#[test]
fn feature_rename_never_touches_the_memory_database() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let ctx = Ctx::new(root);
    create(
        &ctx,
        CreateInput {
            name: "old".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    let sentinel = seed_both(&layout, "api");

    rename(
        &ctx,
        RenameInput {
            feature: "old".to_owned(),
            name: Some("renamed".to_owned()),
            branch: None,
        },
    )
    .unwrap();

    assert_untouched(&layout, &sentinel);
}

#[test]
fn repo_remove_never_touches_the_memory_database() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let origin: Utf8PathBuf =
        seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            origin.as_str(),
            BranchName::new("main").unwrap(),
        )],
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    let ctx = Ctx::new(root);
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    let sentinel = seed_both(&layout, "api");

    remove(
        &ctx,
        RemoveInput {
            name: "api".to_owned(),
            force: false,
        },
    )
    .unwrap();

    assert_untouched(&layout, &sentinel);
}
