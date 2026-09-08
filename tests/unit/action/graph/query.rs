#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::graph::{EdgeKind, Provenance, Span, SymbolKind};

fn setup_test_db() -> (GraphDb, i64, i64, i64) {
    let db = GraphDb::open_in_memory().expect("open in-memory db");
    db.insert_repo("ivar", "/path/to/ivar", "main", None)
        .expect("insert repo");

    // Insert file 1: src/lib.rs
    let file1_id = db
        .upsert_file("ivar", "src/lib.rs", "hash1", 1000, 500)
        .expect("upsert file 1");

    // Insert symbols: helper and caller_fn in src/lib.rs
    let symbols = vec![
        Symbol {
            id: None,
            file_id: Some(file1_id),
            repo: "ivar".to_owned(),
            name: "helper".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn helper()".to_owned()),
            docstring: Some("A helper function".to_owned()),
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: None,
        },
        Symbol {
            id: None,
            file_id: Some(file1_id),
            repo: "ivar".to_owned(),
            name: "caller_fn".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn caller_fn()".to_owned()),
            docstring: Some("A caller function".to_owned()),
            span: Span::new(7, 1, 15, 1),
            is_exported: true,
            complexity: None,
        },
    ];
    let sym_ids = db.insert_symbols(&symbols).expect("insert symbols");
    let helper_id = sym_ids[0];
    let caller_fn_id = sym_ids[1];

    // Insert file 2: src/other.rs
    let file2_id = db
        .upsert_file("ivar", "src/other.rs", "hash2", 2000, 300)
        .expect("upsert file 2");

    // Insert symbol: top_fn in src/other.rs
    let top_syms = vec![Symbol {
        id: None,
        file_id: Some(file2_id),
        repo: "ivar".to_owned(),
        name: "top_fn".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("fn top_fn()".to_owned()),
        docstring: None,
        span: Span::new(1, 1, 10, 1),
        is_exported: false,
        complexity: None,
    }];
    let top_ids = db.insert_symbols(&top_syms).expect("insert top_fn");
    let top_fn_id = top_ids[0];

    // Insert edges:
    // 1. Import edge in src/lib.rs
    // 2. caller_fn -> helper
    // 3. top_fn -> caller_fn
    let edges = vec![
        Edge {
            id: None,
            repo: "ivar".to_owned(),
            file_id: Some(file1_id),
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("std::io".to_owned()),
            kind: EdgeKind::Imports,
            provenance: Provenance::Extracted,
            line: 1,
            col: 1,
            confidence: 1.0,
        },
        Edge {
            id: None,
            repo: "ivar".to_owned(),
            file_id: Some(file1_id),
            from_symbol_id: Some(caller_fn_id),
            to_symbol_id: Some(helper_id),
            to_name: Some("helper".to_owned()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 10,
            col: 5,
            confidence: 0.95,
        },
        Edge {
            id: None,
            repo: "ivar".to_owned(),
            file_id: Some(file2_id),
            from_symbol_id: Some(top_fn_id),
            to_symbol_id: Some(caller_fn_id),
            to_name: Some("caller_fn".to_owned()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 5,
            col: 5,
            confidence: 0.9,
        },
    ];
    db.insert_edges(&edges).expect("insert edges");

    (db, helper_id, caller_fn_id, top_fn_id)
}

#[test]
fn test_find_symbols_exact_and_prefix() {
    let (db, helper_id, caller_fn_id, _) = setup_test_db();

    // Exact match
    let exact = find_symbols(&db, "helper", Some("ivar"), 10).expect("find exact");
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].symbol.id, Some(helper_id));
    assert_eq!(exact[0].symbol.name, "helper");
    assert_eq!(exact[0].file_path, "src/lib.rs");

    // Prefix match
    let prefix = find_symbols(&db, "call", None, 10).expect("find prefix");
    assert_eq!(prefix.len(), 1);
    assert_eq!(prefix[0].symbol.id, Some(caller_fn_id));
    assert_eq!(prefix[0].symbol.name, "caller_fn");

    // Non-existent symbol
    let empty = find_symbols(&db, "non_existent", None, 10).expect("find non existent");
    assert!(empty.is_empty());
}

#[test]
fn test_get_callers() {
    let (db, _helper_id, caller_fn_id, _) = setup_test_db();

    let callers = get_callers(&db, "helper", Some("ivar"), false, 0.5).expect("get callers");
    assert_eq!(callers.len(), 1);
    assert_eq!(callers[0].caller.id, Some(caller_fn_id));
    assert_eq!(callers[0].caller.name, "caller_fn");
    assert_eq!(callers[0].caller_file_path, "src/lib.rs");
    assert_eq!(callers[0].edge_kind, EdgeKind::Calls);
    assert_eq!(callers[0].line, 10);
    assert_eq!(callers[0].col, 5);

    // Filter by high confidence
    let filtered = get_callers(&db, "helper", Some("ivar"), false, 0.99).expect("filtered callers");
    assert!(filtered.is_empty());
}

#[test]
fn test_get_callees() {
    let (db, helper_id, caller_fn_id, _) = setup_test_db();

    let callees = get_callees(&db, caller_fn_id).expect("get callees");
    assert_eq!(callees.len(), 1);
    assert_eq!(callees[0].callee_name, "helper");
    assert_eq!(
        callees[0].callee_symbol.as_ref().and_then(|s| s.id),
        Some(helper_id)
    );
    assert_eq!(callees[0].callee_file_path.as_deref(), Some("src/lib.rs"));
    assert_eq!(callees[0].edge_kind, EdgeKind::Calls);

    // helper has no outgoing calls
    let helper_callees = get_callees(&db, helper_id).expect("helper callees");
    assert!(helper_callees.is_empty());
}

#[test]
fn test_get_file_outline() {
    let (db, _, _, _) = setup_test_db();

    let outline = get_file_outline(&db, "ivar", "src/lib.rs").expect("file outline");
    assert_eq!(outline.file_path, "src/lib.rs");
    assert_eq!(outline.repo, "ivar");
    assert_eq!(outline.symbols.len(), 2);
    assert_eq!(outline.symbols[0].name, "helper");
    assert_eq!(outline.symbols[1].name, "caller_fn");
    assert_eq!(outline.imports.len(), 1);
    assert_eq!(outline.imports[0].to_name.as_deref(), Some("std::io"));

    // File not found error
    let err = get_file_outline(&db, "ivar", "src/missing.rs").unwrap_err();
    match err {
        QueryError::FileNotFound { repo, path } => {
            assert_eq!(repo, "ivar");
            assert_eq!(path, "src/missing.rs");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn test_get_graph_stats() {
    let (db, _, _, _) = setup_test_db();

    let stats = get_graph_stats(&db).expect("get stats");
    assert_eq!(stats.repo_count, 1);
    assert_eq!(stats.file_count, 2);
    assert_eq!(stats.symbol_count, 3);
    assert_eq!(stats.edge_count, 3);
}

#[test]
fn test_get_impact_and_cycle_protection() {
    let (db, helper_id, caller_fn_id, top_fn_id) = setup_test_db();

    // Impact of helper:
    // caller_fn calls helper (depth 1)
    // top_fn calls caller_fn (depth 2)
    let impact = get_impact(&db, helper_id, 5).expect("get impact");
    assert_eq!(impact.root_symbol.id, Some(helper_id));
    assert_eq!(impact.total_affected, 2);
    assert_eq!(impact.affected_symbols.len(), 2);
    assert_eq!(impact.affected_files, vec!["src/lib.rs", "src/other.rs"]);

    assert_eq!(impact.affected_symbols[0].symbol.id, Some(caller_fn_id));
    assert_eq!(impact.affected_symbols[0].depth, 1);
    assert_eq!(
        impact.affected_symbols[0].path_via,
        vec!["helper", "caller_fn"]
    );

    assert_eq!(impact.affected_symbols[1].symbol.id, Some(top_fn_id));
    assert_eq!(impact.affected_symbols[1].depth, 2);
    assert_eq!(
        impact.affected_symbols[1].path_via,
        vec!["helper", "caller_fn", "top_fn"]
    );

    // Impact with max_depth = 1
    let shallow = get_impact(&db, helper_id, 1).expect("shallow impact");
    assert_eq!(shallow.total_affected, 1);
    assert_eq!(shallow.affected_symbols[0].symbol.id, Some(caller_fn_id));

    // Add a cycle: helper calls top_fn
    let file1_id = impact.root_symbol.file_id.unwrap();
    let cycle_edge = Edge {
        id: None,
        repo: "ivar".to_owned(),
        file_id: Some(file1_id),
        from_symbol_id: Some(helper_id),
        to_symbol_id: Some(top_fn_id),
        to_name: Some("top_fn".to_owned()),
        kind: EdgeKind::Calls,
        provenance: Provenance::Extracted,
        line: 3,
        col: 5,
        confidence: 1.0,
    };
    db.insert_edges(&[cycle_edge]).expect("insert cycle edge");

    // Traverse with cycle should terminate cleanly without infinite recursion
    let cycle_impact = get_impact(&db, helper_id, 10).expect("cycle impact");
    assert_eq!(cycle_impact.total_affected, 2);
}
