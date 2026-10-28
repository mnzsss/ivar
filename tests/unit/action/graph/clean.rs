//! Unit tests for graph clean action.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;
use tempfile::tempdir;

use super::*;
use crate::action::Ctx;
use crate::domain::graph::{Edge, EdgeKind, Provenance, Span, Symbol, SymbolKind};
use crate::store::graph::db::GraphDb;
fn populate_hall_with_graph(hall_root: &std::path::Path) -> GraphDb {
    let ivar_dir = hall_root.join(".ivar");
    std::fs::create_dir_all(&ivar_dir).unwrap();
    let db_path = ivar_dir.join("memory.db");
    let db = GraphDb::open(&db_path).unwrap();

    // Insert repos
    db.insert_repo("repo1", "/path1", "main", None).unwrap();
    db.insert_repo("repo2", "/path2", "main", None).unwrap();

    // Insert files
    let f1 = db.upsert_file("repo1", "src/lib.rs", "h1", 1, 100).unwrap();
    let f2 = db
        .upsert_file("repo2", "src/main.rs", "h2", 2, 200)
        .unwrap();

    // Insert symbols
    let s1 = db
        .insert_symbols(&[Symbol {
            id: None,
            file_id: Some(f1),
            repo: "repo1".into(),
            name: "fn1".into(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: None,
        }])
        .unwrap()[0];

    let s2 = db
        .insert_symbols(&[Symbol {
            id: None,
            file_id: Some(f2),
            repo: "repo2".into(),
            name: "fn2".into(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: None,
        }])
        .unwrap()[0];

    // Insert edges
    db.insert_edges(&[
        Edge {
            id: None,
            repo: "repo1".into(),
            file_id: Some(f1),
            from_symbol_id: Some(s1),
            to_symbol_id: None,
            to_name: Some("fn2".into()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 2,
            col: 5,
            confidence: 1.0,
        },
        Edge {
            id: None,
            repo: "repo2".into(),
            file_id: Some(f2),
            from_symbol_id: Some(s2),
            to_symbol_id: None,
            to_name: Some("external".into()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 3,
            col: 5,
            confidence: 1.0,
        },
    ])
    .unwrap();
    // Also write a minimal ivar.json in hall_root so discover_hall succeeds
    let ivar_json = r#"{
        "name": "test-hall",
        "providers": { "available": ["claude-code"], "default": "claude-code" },
        "repos": [
            { "default_branch": "main", "name": "repo1", "url": "https://github.com/example/repo1" },
            { "default_branch": "main", "name": "repo2", "url": "https://github.com/example/repo2" }
        ],
        "version": 1
    }"#;
    std::fs::write(hall_root.join("ivar.json"), ivar_json).unwrap();

    db
}

fn make_ctx(dir: &std::path::Path) -> Ctx {
    Ctx::new(Utf8PathBuf::from_path_buf(dir.to_path_buf()).unwrap())
}

#[test]
fn test_clean_cmd_validation_errors() {
    let dir = tempdir().unwrap();
    populate_hall_with_graph(dir.path());
    let ctx = make_ctx(dir.path());

    // Neither --repo nor --all
    let res = clean_cmd(
        &ctx,
        CleanInput {
            repo: None,
            feature: None,
            all: false,
            path: None,
        },
    );
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(err.code, "graph.clean_target_required");

    // Both --repo and --all
    let res = clean_cmd(
        &ctx,
        CleanInput {
            repo: Some("repo1".into()),
            feature: None,
            all: true,
            path: None,
        },
    );
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(err.code, "graph.clean_conflict");
}

#[test]
fn test_clean_cmd_repo_not_found() {
    let dir = tempdir().unwrap();
    populate_hall_with_graph(dir.path());
    let ctx = make_ctx(dir.path());

    let res = clean_cmd(
        &ctx,
        CleanInput {
            repo: Some("nonexistent".into()),
            feature: None,
            all: false,
            path: None,
        },
    );
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(err.code, "graph.repo_not_found");
}

#[test]
fn test_clean_cmd_removes_repo_and_then_clean_all() {
    let dir = tempdir().unwrap();
    let db = populate_hall_with_graph(dir.path());
    let ctx = make_ctx(dir.path());

    // Initial stats
    let initial_stats = db.stats().unwrap();
    assert_eq!(initial_stats.repo_count, 2);
    assert_eq!(initial_stats.file_count, 2);
    assert_eq!(initial_stats.symbol_count, 2);
    assert_eq!(initial_stats.edge_count, 2);

    // Clean repo1
    let report = clean_cmd(
        &ctx,
        CleanInput {
            repo: Some("repo1".into()),
            feature: None,
            all: false,
            path: None,
        },
    )
    .unwrap();

    assert_eq!(report.value.repo.as_deref(), Some("repo1"));
    assert!(!report.value.all);
    assert_eq!(report.value.repos_removed, 1);
    assert_eq!(report.value.files_removed, 1);
    assert_eq!(report.value.symbols_removed, 1);
    assert_eq!(report.value.edges_removed, 1);

    // DB stats reflect removal
    let mid_stats = db.stats().unwrap();
    assert_eq!(mid_stats.repo_count, 1);
    assert_eq!(mid_stats.file_count, 1);
    assert_eq!(mid_stats.symbol_count, 1);
    assert_eq!(mid_stats.edge_count, 1);

    // Clean all
    let all_report = clean_cmd(
        &ctx,
        CleanInput {
            repo: None,
            feature: None,
            all: true,
            path: None,
        },
    )
    .unwrap();

    assert!(all_report.value.all);
    assert_eq!(all_report.value.repos_removed, 1);
    assert_eq!(all_report.value.files_removed, 1);
    assert_eq!(all_report.value.symbols_removed, 1);
    assert_eq!(all_report.value.edges_removed, 1);
    assert_eq!(all_report.value.usage_removed, 0);

    // Final DB stats
    let final_stats = db.stats().unwrap();
    assert_eq!(final_stats.repo_count, 0);
    assert_eq!(final_stats.file_count, 0);
    assert_eq!(final_stats.symbol_count, 0);
    assert_eq!(final_stats.edge_count, 0);
}

fn hall_with_two_paths() -> (tempfile::TempDir, Utf8PathBuf) {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let ctx = Ctx::new(root.clone());
    crate::action::hall::init(
        &ctx,
        &crate::action::hall::InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
            git: false,
            origin: None,
        },
    )
    .unwrap();
    let layout = crate::store::layout::Layout::at(root.clone());
    let manifest = crate::store::manifest::Manifest::read(&layout)
        .unwrap()
        .unwrap()
        .with_graph_path_added("brand")
        .unwrap()
        .with_graph_path_added("docs")
        .unwrap();
    crate::store::manifest::Manifest::write(&layout, &manifest).unwrap();
    let db = GraphDb::open(root.join(".ivar/memory.db").as_std_path()).unwrap();
    db.insert_repo("hall", root.as_str(), "", None).unwrap();
    db.upsert_file("hall", "brand/DESIGN.md", "h1", 1, 1)
        .unwrap();
    db.upsert_file("hall", "docs/a.md", "h2", 1, 1).unwrap();
    db.watch_register("hall:brand").unwrap();
    db.watch_register("hall:docs").unwrap();
    (tmp, root)
}

fn clean_path(
    root: &Utf8PathBuf,
    path: &str,
) -> Result<crate::action::graph::outcome::CleanOutcome, crate::error::Failure> {
    clean_cmd(
        &Ctx::new(root.clone()),
        CleanInput {
            repo: None,
            feature: None,
            all: false,
            path: Some(path.to_owned()),
        },
    )
    .map(|r| r.value)
}

#[test]
fn clean_path_removes_the_files_the_scope_and_the_manifest_entry() {
    let (_tmp, root) = hall_with_two_paths();

    let outcome = clean_path(&root, "brand/").unwrap();

    assert_eq!(outcome.path.as_deref(), Some("brand"));
    assert_eq!(outcome.files_removed, 1);
    let db = GraphDb::open_read_only(root.join(".ivar/memory.db").as_std_path()).unwrap();
    let paths: Vec<String> = db
        .get_files_for_repo("hall")
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(paths, vec!["docs/a.md"]);
    let scopes: Vec<String> = db
        .watch_scopes()
        .unwrap()
        .into_iter()
        .map(|r| r.scope)
        .collect();
    assert_eq!(scopes, vec!["hall:docs"]);
    let manifest = crate::store::manifest::Manifest::read(&crate::store::layout::Layout::at(root))
        .unwrap()
        .unwrap();
    assert_eq!(manifest.graph_paths(), ["docs"]);
}

#[test]
fn cleaning_the_last_path_drops_the_hall_repo() {
    let (_tmp, root) = hall_with_two_paths();
    clean_path(&root, "brand").unwrap();

    let outcome = clean_path(&root, "docs").unwrap();

    assert_eq!(outcome.repos_removed, 1);
    let db = GraphDb::open_read_only(root.join(".ivar/memory.db").as_std_path()).unwrap();
    assert!(db.get_repo("hall").unwrap().is_none());
}

#[test]
fn clean_path_refuses_a_path_that_is_not_configured() {
    let (_tmp, root) = hall_with_two_paths();

    let error = clean_path(&root, "nope").unwrap_err();

    assert_eq!(error.code, "graph.path_not_configured");
}

#[test]
fn clean_refuses_path_together_with_repo() {
    let (_tmp, root) = hall_with_two_paths();

    let error = clean_cmd(
        &Ctx::new(root),
        CleanInput {
            repo: Some("x".into()),
            feature: None,
            all: false,
            path: Some("brand".into()),
        },
    )
    .unwrap_err();

    assert_eq!(error.code, "graph.clean_conflict");
}

#[test]
fn clean_feature_runs_under_the_index_lock_while_an_unlocked_lock_file_exists() {
    let dir = tempdir().unwrap();
    populate_hall_with_graph(dir.path());
    let lock_path = dir.path().join(".ivar/memory.lock");
    std::fs::write(&lock_path, b"").unwrap();

    let outcome = clean_cmd(
        &make_ctx(dir.path()),
        CleanInput {
            repo: None,
            feature: Some("feat".into()),
            all: false,
            path: None,
        },
    )
    .unwrap()
    .value;

    assert_eq!(outcome.feature.as_deref(), Some("feat"));
    assert_eq!(outcome.repos_removed, 0);
    assert!(lock_path.is_file());
}
