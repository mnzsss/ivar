#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::action::graph::affected::find_affected_tests_with_root;
use crate::action::graph::explore::explore;
use crate::domain::graph::{Span, Symbol, SymbolKind};
use crate::store::graph::db::GraphDb;
use tempfile::tempdir;

#[test]
fn test_explore_reads_source_and_affected_from_promoted_worktree() {
    let temp = tempdir().unwrap();
    let base_dir = temp.path().join("base_core");
    let feat_dir = temp.path().join("feat_core");
    std::fs::create_dir_all(base_dir.join("src")).unwrap();
    std::fs::create_dir_all(feat_dir.join("src")).unwrap();

    std::fs::write(
        base_dir.join("Cargo.toml"),
        "[package]\nname = \"core\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        feat_dir.join("Cargo.toml"),
        "[package]\nname = \"core\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    std::fs::write(
        base_dir.join("src/lib.rs"),
        "pub fn compute() { /* base */ }",
    )
    .unwrap();
    std::fs::write(
        feat_dir.join("src/lib.rs"),
        "pub fn compute() { /* feature worktree modified */ }",
    )
    .unwrap();

    let db = GraphDb::open_in_memory().unwrap();
    db.ensure_views_base_mode().unwrap();
    db.insert_repo("core", base_dir.to_str().unwrap(), "main", Some("c1"))
        .unwrap();
    db.insert_repo("core/1", feat_dir.to_str().unwrap(), "feat", Some("c2"))
        .unwrap();

    let f2 = db
        .upsert_file("core/1", "src/lib.rs", "h2", 200, 200)
        .unwrap();
    db.insert_symbols(&[Symbol {
        id: None,
        file_id: Some(f2),
        repo: "core/1".to_owned(),
        name: "compute".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("pub fn compute()".to_owned()),
        docstring: None,
        span: Span::new(1, 1, 1, 55),
        is_exported: true,
        complexity: Some(1),
    }])
    .unwrap();

    db.configure_session_mode(&[("core", "core/1")]).unwrap();

    // 1. Explore snippet reading
    let res = explore(&db, temp.path(), "compute", None).unwrap();
    assert!(
        !res.primary_symbols.is_empty(),
        "Primary symbols compute must be found"
    );
    assert!(!res.sources.is_empty(), "Sources must be populated");
    let snippet = &res.sources[0].excerpts[0].code;
    assert!(
        snippet.contains("/* feature worktree modified */"),
        "Explore snippet must read from feature worktree: {snippet}"
    );

    // 2. get_visible_repo verification
    let vis_repo = db
        .get_visible_repo("core")
        .unwrap()
        .expect("Visible repo exists");
    assert_eq!(
        vis_repo.root_path,
        feat_dir.to_str().unwrap(),
        "Visible repo root must point to feat_dir"
    );

    // 3. Affected tests command derivation using visible repo root
    let affected = find_affected_tests_with_root(
        &db,
        Some(temp.path()),
        &["src/lib.rs".to_owned()],
        Some("core"),
        5,
    )
    .unwrap();
    assert_eq!(affected.changed_files.len(), 1);
}
