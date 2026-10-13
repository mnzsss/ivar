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
