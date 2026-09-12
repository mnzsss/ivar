#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::domain::graph::{Span, Symbol, SymbolKind};
use crate::store::graph::db::GraphDb;

#[test]
fn test_visible_views_base_and_session_shadowing() {
    let db = GraphDb::open_in_memory().unwrap();
    db.ensure_views_base_mode().unwrap();

    // 1. Seed base repo rows
    db.insert_repo("core", "/base/core", "main", Some("c1"))
        .unwrap();
    let f_base1 = db
        .upsert_file("core", "src/lib.rs", "hash_base", 100, 100)
        .unwrap();
    let f_base2 = db
        .upsert_file("core", "src/deleted.rs", "hash_del", 100, 100)
        .unwrap();
    db.insert_symbols(&[
        Symbol {
            id: None,
            file_id: Some(f_base1),
            repo: "core".to_owned(),
            name: "base_fn".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: None,
        },
        Symbol {
            id: None,
            file_id: Some(f_base2),
            repo: "core".to_owned(),
            name: "old_fn".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: None,
        },
    ])
    .unwrap();

    // In Base mode, visible_files contains both base files, no pseudo-repos
    let base_files: Vec<String> = db
        .conn()
        .prepare("SELECT path FROM visible_files WHERE repo = 'core' ORDER BY path")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(base_files, vec!["src/deleted.rs", "src/lib.rs"]);

    // 2. Seed layer repo rows under pseudo-repo "core/1"
    let layer_id = db
        .ensure_layer_record("feature-a", "core", "/feat/core", "c1")
        .unwrap();
    assert_eq!(layer_id, 1);
    let layer_repo = format!("core/{layer_id}");
    db.insert_repo(&layer_repo, "/feat/core", "feature-a", Some("c1_feat"))
        .unwrap();
    let f_layer1 = db
        .upsert_file(&layer_repo, "src/lib.rs", "hash_feat", 200, 200)
        .unwrap();
    let f_layer3 = db
        .upsert_file(&layer_repo, "src/new.rs", "hash_new", 200, 200)
        .unwrap();
    db.insert_symbols(&[
        Symbol {
            id: None,
            file_id: Some(f_layer1),
            repo: layer_repo.clone(),
            name: "feat_fn".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: None,
        },
        Symbol {
            id: None,
            file_id: Some(f_layer3),
            repo: layer_repo.clone(),
            name: "new_fn".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: None,
        },
    ])
    .unwrap();
    db.set_layer_tombstones(layer_id, &["src/deleted.rs"])
        .unwrap();

    // 3. Switch to Session mode mapping "core" -> "core/1"
    db.configure_session_mode(&[("core", &layer_repo)]).unwrap();

    // visible_files must show src/lib.rs (layer version) and src/new.rs, but NOT src/deleted.rs
    let visible_paths: Vec<String> = db
        .conn()
        .prepare("SELECT path FROM visible_files WHERE repo = 'core' ORDER BY path")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(visible_paths, vec!["src/lib.rs", "src/new.rs"]);

    // visible_symbols must project feat_fn and new_fn under repo 'core' (NOT 'core/1'), and NOT base_fn or old_fn
    let visible_syms: Vec<(String, String)> = db
        .conn()
        .prepare("SELECT repo, name FROM visible_symbols WHERE repo = 'core' ORDER BY name")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(
        visible_syms,
        vec![
            ("core".into(), "feat_fn".into()),
            ("core".into(), "new_fn".into())
        ]
    );
}
