#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::graph::{Edge, EdgeKind, Provenance, Span, Symbol, SymbolKind};
use crate::store::graph::extractor::ExtractedFile;

#[test]
fn test_shortest_path_same_node() -> Result<(), Box<dyn std::error::Error>> {
    let db = GraphDb::open_in_memory()?;
    db.insert_repo("repo", "/root", "main", None)?;

    let file_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "repo".to_owned(),
            name: "Solo".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn Solo()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
        }],
        edges: vec![],
    };
    db.index_extracted_file(
        "repo",
        "src/solo.rs",
        "hash_solo",
        100,
        100,
        &file_extracted,
    )?;

    let res = find_shortest_path(&db, "Solo", "Solo", 5)?;
    assert!(res.is_some());
    let path = res.unwrap();
    assert_eq!(path.from, "Solo");
    assert_eq!(path.to, "Solo");
    assert!(path.steps.is_empty());

    Ok(())
}

#[test]
fn test_shortest_path_not_found() -> Result<(), Box<dyn std::error::Error>> {
    let db = GraphDb::open_in_memory()?;
    db.insert_repo("repo", "/root", "main", None)?;

    let res = find_shortest_path(&db, "MissingA", "MissingB", 5);
    assert!(matches!(res, Err(PathError::StartNotFound(_))));

    Ok(())
}

#[test]
fn test_shortest_path_bidirectional_chain() -> Result<(), Box<dyn std::error::Error>> {
    let db = GraphDb::open_in_memory()?;
    db.insert_repo("repo", "/root", "main", None)?;

    // Chain: A -> B -> C -> D
    let file_extracted = ExtractedFile {
        symbols: vec![
            Symbol {
                id: None,
                file_id: None,
                repo: "repo".to_owned(),
                name: "A".to_owned(),
                kind: SymbolKind::Fn,
                scope: None,
                signature: Some("fn A()".to_owned()),
                docstring: None,
                span: Span::new(1, 1, 5, 1),
                is_exported: true,
            },
            Symbol {
                id: None,
                file_id: None,
                repo: "repo".to_owned(),
                name: "B".to_owned(),
                kind: SymbolKind::Fn,
                scope: None,
                signature: Some("fn B()".to_owned()),
                docstring: None,
                span: Span::new(6, 1, 10, 1),
                is_exported: true,
            },
            Symbol {
                id: None,
                file_id: None,
                repo: "repo".to_owned(),
                name: "C".to_owned(),
                kind: SymbolKind::Fn,
                scope: None,
                signature: Some("fn C()".to_owned()),
                docstring: None,
                span: Span::new(11, 1, 15, 1),
                is_exported: true,
            },
            Symbol {
                id: None,
                file_id: None,
                repo: "repo".to_owned(),
                name: "D".to_owned(),
                kind: SymbolKind::Fn,
                scope: None,
                signature: Some("fn D()".to_owned()),
                docstring: None,
                span: Span::new(16, 1, 20, 1),
                is_exported: true,
            },
        ],
        edges: vec![
            Edge {
                id: None,
                repo: "repo".to_owned(),
                file_id: None,
                from_symbol_id: None,
                to_symbol_id: None,
                to_name: Some("B".to_owned()),
                kind: EdgeKind::Calls,
                provenance: Provenance::Extracted,
                line: 2,
                col: 4,
                confidence: 1.0,
            },
            Edge {
                id: None,
                repo: "repo".to_owned(),
                file_id: None,
                from_symbol_id: None,
                to_symbol_id: None,
                to_name: Some("C".to_owned()),
                kind: EdgeKind::Calls,
                provenance: Provenance::Extracted,
                line: 7,
                col: 4,
                confidence: 1.0,
            },
            Edge {
                id: None,
                repo: "repo".to_owned(),
                file_id: None,
                from_symbol_id: None,
                to_symbol_id: None,
                to_name: Some("D".to_owned()),
                kind: EdgeKind::Calls,
                provenance: Provenance::Extracted,
                line: 12,
                col: 4,
                confidence: 1.0,
            },
        ],
    };

    db.index_extracted_file(
        "repo",
        "src/main.rs",
        "hash_main",
        100,
        100,
        &file_extracted,
    )?;

    let conn = db.conn();
    let sym_a: i64 = conn.query_row("SELECT id FROM symbols WHERE name = 'A'", [], |r| r.get(0))?;
    let sym_b: i64 = conn.query_row("SELECT id FROM symbols WHERE name = 'B'", [], |r| r.get(0))?;
    let sym_c: i64 = conn.query_row("SELECT id FROM symbols WHERE name = 'C'", [], |r| r.get(0))?;
    let sym_d: i64 = conn.query_row("SELECT id FROM symbols WHERE name = 'D'", [], |r| r.get(0))?;

    conn.execute(
        "UPDATE edges SET from_symbol_id = ?1, to_symbol_id = ?2 WHERE to_name = 'B'",
        params![sym_a, sym_b],
    )?;
    conn.execute(
        "UPDATE edges SET from_symbol_id = ?1, to_symbol_id = ?2 WHERE to_name = 'C'",
        params![sym_b, sym_c],
    )?;
    conn.execute(
        "UPDATE edges SET from_symbol_id = ?1, to_symbol_id = ?2 WHERE to_name = 'D'",
        params![sym_c, sym_d],
    )?;

    let res = find_shortest_path(&db, "A", "D", 5)?;
    assert!(res.is_some());
    let path = res.unwrap();
    assert_eq!(path.from, "A");
    assert_eq!(path.to, "D");
    assert_eq!(path.steps.len(), 3);
    assert_eq!(path.steps[0].source, "A");
    assert_eq!(path.steps[0].target, "B");
    assert_eq!(path.steps[1].source, "B");
    assert_eq!(path.steps[1].target, "C");
    assert_eq!(path.steps[2].source, "C");
    assert_eq!(path.steps[2].target, "D");

    // Max hops exceeded check
    let res_hops = find_shortest_path(&db, "A", "D", 2)?;
    assert!(res_hops.is_none());

    Ok(())
}
