//! Unit tests for cyclomatic complexity analysis action.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::graph::{Span, Symbol, SymbolKind};
use crate::store::graph::extractor::ExtractedFile;

#[test]
fn test_execute_complexity_ranking_and_threshold() {
    let db = GraphDb::open_in_memory().expect("open db");
    db.insert_repo("test_repo", "/path", "main", None).unwrap();

    let extracted = ExtractedFile {
        symbols: vec![
            Symbol {
                id: None,
                file_id: None,
                repo: "test_repo".to_owned(),
                name: "low_complexity".to_owned(),
                kind: SymbolKind::Fn,
                scope: None,
                signature: Some("fn low_complexity()".to_owned()),
                docstring: None,
                span: Span::new(1, 1, 10, 1),
                is_exported: true,
                complexity: Some(3),
            },
            Symbol {
                id: None,
                file_id: None,
                repo: "test_repo".to_owned(),
                name: "high_complexity".to_owned(),
                kind: SymbolKind::Fn,
                scope: None,
                signature: Some("fn high_complexity()".to_owned()),
                docstring: None,
                span: Span::new(20, 1, 50, 1),
                is_exported: true,
                complexity: Some(25),
            },
            Symbol {
                id: None,
                file_id: None,
                repo: "test_repo".to_owned(),
                name: "mid_complexity".to_owned(),
                kind: SymbolKind::Fn,
                scope: None,
                signature: Some("fn mid_complexity()".to_owned()),
                docstring: None,
                span: Span::new(60, 1, 80, 1),
                is_exported: false,
                complexity: Some(15),
            },
        ],
        edges: vec![],
    };

    db.index_extracted_file(
        "test_repo",
        "src/compute.rs",
        "h_compute",
        1,
        100,
        &extracted,
    )
    .unwrap();

    // Query with threshold 10
    let items = execute_complexity(&db, Some("test_repo"), 10, 10).expect("execute_complexity");

    // Must be sorted descending: high_complexity (25), mid_complexity (15)
    assert_eq!(items.len(), 2);
    let first = items.first().expect("first item");
    assert_eq!(first.symbol.name, "high_complexity");
    assert_eq!(first.complexity, 25);
    assert_eq!(first.file_path, "src/compute.rs");
    assert_eq!(first.line, 20);

    let second = items.get(1).expect("second item");
    assert_eq!(second.symbol.name, "mid_complexity");
    assert_eq!(second.complexity, 15);
    assert_eq!(second.file_path, "src/compute.rs");
    assert_eq!(second.line, 60);

    // Query with threshold 20
    let high_only = execute_complexity(&db, Some("test_repo"), 20, 10).expect("execute_complexity");
    assert_eq!(high_only.len(), 1);
    assert_eq!(
        high_only
            .first()
            .expect("first high complexity item")
            .symbol
            .name,
        "high_complexity"
    );
}
