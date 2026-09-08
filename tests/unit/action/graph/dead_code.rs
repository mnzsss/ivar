//! Unit tests for dead code analysis action.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::graph::{Edge, EdgeKind, Provenance, Span, Symbol, SymbolKind};
use crate::store::graph::extractor::ExtractedFile;

#[test]
fn test_execute_dead_code_identifies_unreferenced_private_symbol() {
    let db = GraphDb::open_in_memory().expect("open db");
    db.insert_repo("test_repo", "/path", "main", None).unwrap();

    // 1. src/utils.rs defines private helper `unused_helper` and public helper `public_helper`
    let utils_extracted = ExtractedFile {
        symbols: vec![
            Symbol {
                id: None,
                file_id: None,
                repo: "test_repo".to_owned(),
                name: "unused_helper".to_owned(),
                kind: SymbolKind::Fn,
                scope: None,
                signature: Some("fn unused_helper()".to_owned()),
                docstring: None,
                span: Span::new(10, 1, 20, 1),
                is_exported: false,
                complexity: None,
            },
            Symbol {
                id: None,
                file_id: None,
                repo: "test_repo".to_owned(),
                name: "public_helper".to_owned(),
                kind: SymbolKind::Fn,
                scope: None,
                signature: Some("pub fn public_helper()".to_owned()),
                docstring: None,
                span: Span::new(25, 1, 35, 1),
                is_exported: true,
                complexity: None,
            },
        ],
        edges: vec![],
    };

    db.index_extracted_file(
        "test_repo",
        "src/utils.rs",
        "h_utils",
        1,
        100,
        &utils_extracted,
    )
    .unwrap();

    // 2. tests/test_helper.rs defines a test helper (in a test file)
    let test_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "test_repo".to_owned(),
            name: "test_only_private".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn test_only_private()".to_owned()),
            docstring: None,
            span: Span::new(5, 1, 15, 1),
            is_exported: false,
            complexity: None,
        }],
        edges: vec![],
    };

    db.index_extracted_file(
        "test_repo",
        "tests/test_helper.rs",
        "h_test",
        1,
        100,
        &test_extracted,
    )
    .unwrap();

    // 3. src/caller.rs defines `called_private_fn` and calls it
    let caller_extracted = ExtractedFile {
        symbols: vec![
            Symbol {
                id: None,
                file_id: None,
                repo: "test_repo".to_owned(),
                name: "caller_fn".to_owned(),
                kind: SymbolKind::Fn,
                scope: None,
                signature: Some("pub fn caller_fn()".to_owned()),
                docstring: None,
                span: Span::new(1, 1, 10, 1),
                is_exported: true,
                complexity: None,
            },
            Symbol {
                id: None,
                file_id: None,
                repo: "test_repo".to_owned(),
                name: "called_private_fn".to_owned(),
                kind: SymbolKind::Fn,
                scope: None,
                signature: Some("fn called_private_fn()".to_owned()),
                docstring: None,
                span: Span::new(12, 1, 20, 1),
                is_exported: false,
                complexity: None,
            },
        ],
        edges: vec![Edge {
            id: None,
            repo: "test_repo".to_owned(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("called_private_fn".to_owned()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 5,
            col: 9,
            confidence: 1.0,
        }],
    };

    db.index_extracted_file(
        "test_repo",
        "src/caller.rs",
        "h_caller",
        1,
        100,
        &caller_extracted,
    )
    .unwrap();

    // Execute dead code analysis
    let dead_items = execute_dead_code(&db, Some("test_repo"), 10).expect("execute_dead_code");

    // Assert: `unused_helper` is found
    assert_eq!(dead_items.len(), 1);
    let first = dead_items.first().expect("first dead item");
    assert_eq!(first.symbol.name, "unused_helper");
    assert_eq!(first.file_path, "src/utils.rs");
    assert_eq!(first.line, 10);
}
