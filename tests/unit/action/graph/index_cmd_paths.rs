#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use camino::Utf8PathBuf;

use crate::action::Ctx;
use crate::action::graph::index_cmd;
use crate::action::graph::input::IndexInput;
use crate::action::graph::outcome::IndexBatchOutcome;
use crate::action::hall::{InitInput, init};
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

fn hall_with_brand() -> (tempfile::TempDir, Utf8PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let ctx = Ctx::new(root.clone());
    init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
            git: false,
            origin: None,
        },
    )
    .unwrap();
    std::fs::create_dir_all(root.join("brand")).unwrap();
    std::fs::write(root.join("brand/DESIGN.md"), "# design\n").unwrap();
    (tmp, root)
}

fn input(paths: &[&str]) -> IndexInput {
    IndexInput {
        repo: None,
        full: false,
        register_mcp: false,
        paths: paths.iter().map(|p| (*p).to_owned()).collect(),
    }
}

fn index(root: &Utf8PathBuf, paths: &[&str]) -> IndexBatchOutcome {
    index_cmd(&Ctx::new(root.clone()), input(paths))
        .unwrap()
        .value
}

#[test]
fn index_with_a_path_persists_it_and_indexes_it_under_repo_hall() {
    let (_tmp, root) = hall_with_brand();

    let outcome = index(&root, &["brand/"]);

    assert!(
        outcome
            .repos
            .iter()
            .any(|r| r.repo == "hall" && r.files_indexed == 1),
        "{outcome:?}"
    );
    let manifest = Manifest::read(&Layout::at(root.clone())).unwrap().unwrap();
    assert_eq!(manifest.graph_paths(), ["brand"]);
    let db = GraphDb::open_read_only(root.join(".ivar/memory.db").as_std_path()).unwrap();
    assert!(db.get_file("hall", "brand/DESIGN.md").unwrap().is_some());
}

#[test]
fn a_bare_index_reindexes_every_configured_path_and_keeps_its_scope() {
    let (_tmp, root) = hall_with_brand();
    index(&root, &["brand"]);
    std::fs::write(root.join("brand/NEW.md"), "new\n").unwrap();
    {
        let db = GraphDb::open(root.join(".ivar/memory.db").as_std_path()).unwrap();
        db.watch_register("hall:brand").unwrap();
    }

    let outcome = index(&root, &[]);

    assert!(
        outcome
            .repos
            .iter()
            .any(|r| r.repo == "hall" && r.files_indexed >= 1),
        "{outcome:?}"
    );
    assert!(outcome.removed.scopes.is_empty(), "{:?}", outcome.removed);

    let again = index(&root, &[]);

    let hall = again
        .repos
        .iter()
        .find(|r| r.repo == "hall")
        .unwrap_or_else(|| panic!("{again:?}"));
    assert_eq!(hall.files_indexed, 0, "{again:?}");
    assert_eq!(hall.files_deleted, 0, "{again:?}");
    assert!(again.removed.is_empty(), "{:?}", again.removed);
}

#[test]
fn index_with_a_path_reconciles_nothing() {
    let (_tmp, root) = hall_with_brand();
    {
        let db = GraphDb::open(root.join(".ivar/memory.db").as_std_path()).unwrap();
        db.insert_repo("orca", "/x/orca", "main", None).unwrap();
    }

    let outcome = index(&root, &["brand"]);

    assert!(outcome.removed.is_empty(), "{:?}", outcome.removed);
    let db = GraphDb::open_read_only(root.join(".ivar/memory.db").as_std_path()).unwrap();
    assert!(db.get_repo("orca").unwrap().is_some());
}

#[test]
fn index_refuses_a_path_inside_ivar_and_writes_nothing() {
    let (_tmp, root) = hall_with_brand();

    let error = index_cmd(&Ctx::new(root.clone()), input(&[".ivar"])).unwrap_err();

    assert_eq!(error.code, "graph.path_invalid");
    let manifest = Manifest::read(&Layout::at(root)).unwrap().unwrap();
    assert!(manifest.graph_paths().is_empty());
}

#[test]
fn index_refuses_repo_with_path_and_writes_nothing() {
    let (_tmp, root) = hall_with_brand();

    let error = index_cmd(
        &Ctx::new(root.clone()),
        IndexInput {
            repo: Some("x".to_owned()),
            ..input(&["brand"])
        },
    )
    .unwrap_err();

    assert_eq!(error.code, "graph.index_conflict");
    let on_disk = std::fs::read_to_string(root.join("ivar.json")).unwrap();
    assert!(!on_disk.contains("\"graph\""), "{on_disk}");
}

fn hall_with_synced_api() -> (tempfile::TempDir, Utf8PathBuf) {
    let (tmp, root) = crate::test_support::hall_root();
    let ctx = Ctx::new(root.clone());
    init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
            git: false,
            origin: None,
        },
    )
    .unwrap();
    let origin = crate::test_support::seeded_repo(
        &root.parent().unwrap().join("origins").join("api"),
        "main",
    );
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout)
        .unwrap()
        .unwrap()
        .with_repo_added(crate::store::manifest::Repo::new(
            crate::domain::name::RepoName::new("api").unwrap(),
            origin.as_str(),
            crate::domain::name::BranchName::new("main").unwrap(),
        ))
        .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    (tmp, root)
}

#[test]
fn index_of_one_repo_reconciles_nothing() {
    let (_tmp, root) = hall_with_synced_api();
    {
        let db = GraphDb::open(root.join(".ivar/memory.db").as_std_path()).unwrap();
        db.insert_repo("orca", "/x/orca", "main", None).unwrap();
    }

    let outcome = index_cmd(
        &Ctx::new(root.clone()),
        IndexInput {
            repo: Some("api".to_owned()),
            ..input(&[])
        },
    )
    .unwrap()
    .value;

    assert!(outcome.repos.iter().any(|r| r.repo == "api"), "{outcome:?}");
    assert!(outcome.removed.is_empty(), "{:?}", outcome.removed);
    let db = GraphDb::open_read_only(root.join(".ivar/memory.db").as_std_path()).unwrap();
    assert!(db.get_repo("orca").unwrap().is_some());
}
