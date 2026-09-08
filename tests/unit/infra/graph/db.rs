#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use tempfile::tempdir;

#[test]
fn test_open_in_memory_and_stats() {
    let db = GraphDb::open_in_memory().expect("failed to open in memory db");
    let stats = db.stats().expect("failed to get stats");
    assert_eq!(stats.repo_count, 0);
    assert_eq!(stats.file_count, 0);
    assert_eq!(stats.symbol_count, 0);
    assert_eq!(stats.edge_count, 0);
}

#[test]
fn test_open_on_disk() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("sub").join("graph.db");
    let db = GraphDb::open(&db_path).expect("failed to open on-disk db");
    let stats = db.stats().expect("stats");
    assert_eq!(stats.repo_count, 0);

    // Verify foreign_keys enabled
    let fk: i64 = db
        .conn()
        .query_row("PRAGMA foreign_keys;", [], |r| r.get(0))
        .expect("pragma fk");
    assert_eq!(fk, 1);
}

#[test]
fn test_repo_and_file_crud() {
    let db = GraphDb::open_in_memory().expect("open");
    db.insert_repo("ivar", "/path/to/ivar", "main", Some("abcdef012345"))
        .expect("insert repo");

    let repo = db.get_repo("ivar").expect("get repo").expect("exists");
    assert_eq!(repo.id, "ivar");
    assert_eq!(repo.root_path, "/path/to/ivar");
    assert_eq!(repo.default_branch, "main");
    assert_eq!(repo.last_indexed_commit.as_deref(), Some("abcdef012345"));

    db.update_repo_commit("ivar", "112233445566")
        .expect("update repo commit");
    let repo_updated = db.get_repo("ivar").expect("get repo").expect("exists");
    assert_eq!(
        repo_updated.last_indexed_commit.as_deref(),
        Some("112233445566")
    );

    let file_id = db
        .upsert_file("ivar", "src/main.rs", "hash_abc", 1000, 2048)
        .expect("upsert file");
    assert!(file_id > 0);

    let file = db
        .get_file("ivar", "src/main.rs")
        .expect("get file")
        .expect("exists");
    assert_eq!(file.id, file_id);
    assert_eq!(file.repo, "ivar");
    assert_eq!(file.path, "src/main.rs");
    assert_eq!(file.content_hash, "hash_abc");

    // Re-upsert file updates existing row
    let file_id_2 = db
        .upsert_file("ivar", "src/main.rs", "hash_def", 2000, 4096)
        .expect("upsert updated file");
    assert_eq!(file_id, file_id_2);

    let file2 = db
        .get_file("ivar", "src/main.rs")
        .expect("get file")
        .expect("exists");
    assert_eq!(file2.content_hash, "hash_def");
    assert_eq!(file2.size_bytes, 4096);
}

#[test]
fn test_foreign_key_cascades() {
    let db = GraphDb::open_in_memory().expect("open");
    db.insert_repo("ivar", "/path", "main", None).unwrap();
    let file_id = db
        .upsert_file("ivar", "src/lib.rs", "hash1", 1, 100)
        .unwrap();

    let sym = Symbol {
        id: None,
        file_id: Some(file_id),
        repo: "ivar".to_string(),
        name: "init".to_string(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("pub fn init()".to_string()),
        docstring: Some("Initializes the system.".to_string()),
        span: Span::new(1, 1, 10, 1),
        is_exported: true,
    };
    let sym_ids = db.insert_symbols(&[sym]).unwrap();
    let sym_id = sym_ids[0];

    let edge = Edge {
        id: None,
        repo: "ivar".to_string(),
        file_id: Some(file_id),
        from_symbol_id: Some(sym_id),
        to_symbol_id: None,
        to_name: Some("sub_init".to_string()),
        kind: EdgeKind::Calls,
        provenance: Provenance::Extracted,
        line: 5,
        col: 9,
        confidence: 1.0,
    };
    db.insert_edges(&[edge]).unwrap();

    let stats = db.stats().unwrap();
    assert_eq!(stats.file_count, 1);
    assert_eq!(stats.symbol_count, 1);
    assert_eq!(stats.edge_count, 1);

    // Deleting file should cascade delete symbol and edge
    db.delete_file("ivar", "src/lib.rs").unwrap();

    let stats_after = db.stats().unwrap();
    assert_eq!(stats_after.file_count, 0);
    assert_eq!(stats_after.symbol_count, 0);
    assert_eq!(stats_after.edge_count, 0);
}

#[test]
fn test_delete_symbols_for_file() {
    let db = GraphDb::open_in_memory().expect("open");
    db.insert_repo("ivar", "/path", "main", None).unwrap();
    let file_id = db
        .upsert_file("ivar", "src/lib.rs", "hash1", 1, 100)
        .unwrap();

    let sym = Symbol {
        id: None,
        file_id: Some(file_id),
        repo: "ivar".to_string(),
        name: "test_fn".to_string(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: None,
        docstring: None,
        span: Span::new(1, 1, 5, 1),
        is_exported: false,
    };
    let sym_ids = db.insert_symbols(&[sym]).unwrap();
    let sym_id = sym_ids[0];

    let edge = Edge {
        id: None,
        repo: "ivar".to_string(),
        file_id: Some(file_id),
        from_symbol_id: Some(sym_id),
        to_symbol_id: None,
        to_name: Some("callee".to_string()),
        kind: EdgeKind::Calls,
        provenance: Provenance::Extracted,
        line: 2,
        col: 5,
        confidence: 1.0,
    };
    db.insert_edges(&[edge]).unwrap();

    db.delete_symbols_for_file(file_id).unwrap();

    let stats = db.stats().unwrap();
    assert_eq!(stats.file_count, 1);
    assert_eq!(stats.symbol_count, 0);
    assert_eq!(stats.edge_count, 0);
}

#[test]
fn test_fts5_triggers_and_search() {
    let db = GraphDb::open_in_memory().expect("open");
    db.insert_repo("ivar", "/path", "main", None).unwrap();
    let file_id = db
        .upsert_file("ivar", "src/service.rs", "hash_srv", 1, 500)
        .unwrap();

    let sym = Symbol {
        id: None,
        file_id: Some(file_id),
        repo: "ivar".to_string(),
        name: "compute_hash".to_string(),
        kind: SymbolKind::Fn,
        scope: Some("crate::service".to_string()),
        signature: Some("pub fn compute_hash(data: &[u8]) -> String".to_string()),
        docstring: Some("Computes SHA256 digest of input payload.".to_string()),
        span: Span::new(10, 1, 25, 1),
        is_exported: true,
    };
    db.insert_symbols(&[sym]).unwrap();

    // Search by symbol name
    let results = db.search_symbols_fts("compute_hash", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "compute_hash");
    assert_eq!(results[0].kind, SymbolKind::Fn);
    assert!(results[0].is_exported);

    // Search by docstring token
    let results_doc = db.search_symbols_fts("SHA256", 10).unwrap();
    assert_eq!(results_doc.len(), 1);
    assert_eq!(results_doc[0].name, "compute_hash");

    // Search by signature token
    let results_sig = db.search_symbols_fts("payload", 10).unwrap();
    assert_eq!(results_sig.len(), 1);

    // Delete symbol and check FTS index updated
    db.delete_symbols_for_file(file_id).unwrap();
    let results_after = db.search_symbols_fts("compute_hash", 10).unwrap();
    assert_eq!(results_after.len(), 0);
}

#[test]
fn test_relink_dangling_edges() {
    let db = GraphDb::open_in_memory().expect("open");
    db.insert_repo("ivar", "/path", "main", None).unwrap();
    let file_caller = db
        .upsert_file("ivar", "src/caller.rs", "h1", 1, 100)
        .unwrap();
    let file_callee = db
        .upsert_file("ivar", "src/callee.rs", "h2", 1, 100)
        .unwrap();

    // Insert edge before callee symbol exists (dangling edge with to_symbol_id = NULL)
    let edge = Edge {
        id: None,
        repo: "ivar".to_string(),
        file_id: Some(file_caller),
        from_symbol_id: None,
        to_symbol_id: None,
        to_name: Some("target_fn".to_string()),
        kind: EdgeKind::Calls,
        provenance: Provenance::Extracted,
        line: 4,
        col: 10,
        confidence: 1.0,
    };
    db.insert_edges(&[edge]).unwrap();

    // Now insert target symbol
    let target_sym = Symbol {
        id: None,
        file_id: Some(file_callee),
        repo: "ivar".to_string(),
        name: "target_fn".to_string(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("pub fn target_fn()".to_string()),
        docstring: None,
        span: Span::new(1, 1, 10, 1),
        is_exported: true,
    };
    let sym_ids = db.insert_symbols(&[target_sym]).unwrap();
    let target_sym_id = sym_ids[0];

    let relinked = db.relink_dangling_edges("ivar").unwrap();
    assert_eq!(relinked, 1);

    // Verify edge now points to target_sym_id
    let to_sym_id: Option<i64> = db
        .conn()
        .query_row(
            "SELECT to_symbol_id FROM edges WHERE repo = 'ivar' AND to_name = 'target_fn'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(to_sym_id, Some(target_sym_id));
}
