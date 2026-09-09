//! Unit tests for graph clean action.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
    let f2 = db.upsert_file("repo2", "src/main.rs", "h2", 2, 200).unwrap();

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
        "schema_version": "1.0",
        "hall": "test-hall",
        "repos": {
            "repo1": { "url": "https://github.com/example/repo1" },
            "repo2": { "url": "https://github.com/example/repo2" }
        }
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
            all: false,
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
            all: true,
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
            all: false,
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
            all: false,
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
            all: true,
        },
    )
    .unwrap();

    assert!(all_report.value.all);
    assert_eq!(all_report.value.repos_removed, 1);
    assert_eq!(all_report.value.files_removed, 1);
    assert_eq!(all_report.value.symbols_removed, 1);
    assert_eq!(all_report.value.edges_removed, 1);

    // Final DB stats
    let final_stats = db.stats().unwrap();
    assert_eq!(final_stats.repo_count, 0);
    assert_eq!(final_stats.file_count, 0);
    assert_eq!(final_stats.symbol_count, 0);
    assert_eq!(final_stats.edge_count, 0);
}
