//! Unit tests for standalone HTML code graph visualizer generator.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use tempfile::tempdir;

use super::*;
use crate::domain::graph::{Edge, EdgeKind, Provenance, Span, Symbol, SymbolKind};
use crate::store::graph::extractor::ExtractedFile;

#[test]
fn test_generate_html_structure_and_no_external_links() {
    let data = VizData {
        nodes: vec![
            VizNode {
                id: 1,
                name: "Foo".to_owned(),
                kind: "struct".to_owned(),
                file: "src/foo.rs".to_owned(),
                repo: "test_repo".to_owned(),
                line: 10,
                complexity: Some(5),
                is_exported: true,
            },
            VizNode {
                id: 2,
                name: "bar".to_owned(),
                kind: "fn".to_owned(),
                file: "src/foo.rs".to_owned(),
                repo: "test_repo".to_owned(),
                line: 25,
                complexity: None,
                is_exported: false,
            },
        ],
        edges: vec![VizEdge {
            from: 1,
            to: 2,
            kind: "calls".to_owned(),
        }],
    };

    let html = generate_html(&data).expect("generate html");

    assert!(html.contains("<!DOCTYPE html>"));
    assert!(html.contains("<div id=\"cy\"></div>") || html.contains("id=\"cy\""));
    assert!(html.contains("<style>"));
    assert!(html.contains("<script>"));
    assert!(html.contains("cytoscape("));
    assert!(html.contains("const ELEMENTS ="));
    assert!(html.contains(r#""id": "repo:test_repo""#) || html.contains(r#""id":"repo:test_repo""#));
    assert!(html.contains(r#""id": "file:test_repo:src/foo.rs""#) || html.contains(r#""id":"file:test_repo:src/foo.rs""#));
    assert!(html.contains(r#""parent": "repo:test_repo""#) || html.contains(r#""parent":"repo:test_repo""#));
    assert!(html.contains(r#""parent": "file:test_repo:src/foo.rs""#) || html.contains(r#""parent":"file:test_repo:src/foo.rs""#));
    assert!(html.contains(r#""id": "sym:1""#) || html.contains(r#""id":"sym:1""#));
    assert!(html.contains(r#""id": "sym:2""#) || html.contains(r#""id":"sym:2""#));
    assert!(html.contains(r#""id": "e:1-\u003e2""#) || html.contains(r#""id":"e:1-\u003e2""#) || html.contains("e:1"));
    assert!(html.contains(r#""label": "Foo""#) || html.contains(r#""label":"Foo""#));
    assert!(html.contains(r#""label": "bar""#) || html.contains(r#""label":"bar""#));
    assert!(html.contains(r#""kind": "calls""#) || html.contains(r#""kind":"calls""#));

    // Verify generated HTML contains zero external network requests
    assert!(!html.contains("http://"), "HTML must not contain http://");
    assert!(!html.contains("https://"), "HTML must not contain https://");
    assert!(!html.contains("//unpkg.com"), "HTML must not contain unpkg");
    assert!(!html.contains("//cdnjs"), "HTML must not contain cdnjs");
    assert!(!html.contains("//cdn.jsdelivr.net"), "HTML must not contain jsdelivr");
}

#[test]
fn test_generate_html_escapes_script_tags() {
    let data = VizData {
        nodes: vec![VizNode {
            id: 1,
            name: "</script><script>alert('xss')</script>".to_owned(),
            kind: "Function".to_owned(),
            file: "src/hack.rs".to_owned(),
            line: 1,
            complexity: None,
            is_exported: true,
            repo: "default".to_owned(),
        }],
        edges: vec![],
    };

    let html = generate_html(&data).expect("generate html");
    assert!(!html.contains("</script><script>"));
    assert!(html.contains(r#"\u003c/script\u003e"#));
}

#[test]
fn test_collect_viz_data_and_repo_filter() {
    let db = GraphDb::open_in_memory().expect("open db");
    db.insert_repo("repo_a", "/path/a", "main", None).unwrap();
    db.insert_repo("repo_b", "/path/b", "main", None).unwrap();

    let extracted_a = ExtractedFile {
        symbols: vec![
            Symbol {
                id: None,
                file_id: None,
                repo: "repo_a".to_owned(),
                name: "Alpha".to_owned(),
                kind: SymbolKind::Fn,
                scope: None,
                signature: None,
                docstring: None,
                span: Span::new(1, 1, 10, 1),
                is_exported: true,
                complexity: Some(3),
            },
            Symbol {
                id: None,
                file_id: None,
                repo: "repo_a".to_owned(),
                name: "Beta".to_owned(),
                kind: SymbolKind::Fn,
                scope: None,
                signature: None,
                docstring: None,
                span: Span::new(12, 1, 20, 1),
                is_exported: false,
                complexity: None,
            },
        ],
        edges: vec![Edge {
            id: None,
            repo: "repo_a".to_owned(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("Beta".to_owned()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 5,
            col: 1,
            confidence: 1.0,
        }],
    };

    let extracted_b = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "repo_b".to_owned(),
            name: "Gamma".to_owned(),
            kind: SymbolKind::Struct,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(1, 1, 15, 1),
            is_exported: true,
            complexity: Some(8),
        }],
        edges: vec![],
    };

    db.index_extracted_file("repo_a", "src/a.rs", "hash_a", 1, 100, &extracted_a)
        .unwrap();
    db.index_extracted_file("repo_b", "src/b.rs", "hash_b", 1, 200, &extracted_b)
        .unwrap();

    // 1. All repos
    let all_data = collect_viz_data(&db, None).expect("collect all data");
    assert_eq!(all_data.nodes.len(), 3);
    assert_eq!(all_data.edges.len(), 1);

    let alpha_node = all_data
        .nodes
        .iter()
        .find(|n| n.name == "Alpha")
        .expect("Alpha node");
    assert_eq!(alpha_node.repo, "repo_a");
    assert_eq!(alpha_node.file, "src/a.rs");
    assert_eq!(alpha_node.line, 1);
    assert_eq!(alpha_node.complexity, Some(3));
    assert!(alpha_node.is_exported);

    let beta_node = all_data
        .nodes
        .iter()
        .find(|n| n.name == "Beta")
        .expect("Beta node");
    assert_eq!(beta_node.complexity, None);
    assert!(!beta_node.is_exported);

    // 2. Filter repo_a
    let repo_a_data = collect_viz_data(&db, Some("repo_a")).expect("collect repo_a data");
    assert_eq!(repo_a_data.nodes.len(), 2);
    assert_eq!(repo_a_data.edges.len(), 1);
    assert!(repo_a_data.nodes.iter().all(|n| n.repo == "repo_a"));

    // 3. Filter repo_b
    let repo_b_data = collect_viz_data(&db, Some("repo_b")).expect("collect repo_b data");
    assert_eq!(repo_b_data.nodes.len(), 1);
    assert_eq!(repo_b_data.edges.len(), 0);
    assert_eq!(
        repo_b_data.nodes.first().expect("repo_b first node").name,
        "Gamma"
    );
}

#[test]
fn test_execute_viz_writes_file_to_disk() {
    let db = GraphDb::open_in_memory().expect("open db");
    db.insert_repo("repo_1", "/path/1", "main", None).unwrap();

    let extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "repo_1".to_owned(),
            name: "Server".to_owned(),
            kind: SymbolKind::Class,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(1, 1, 50, 1),
            is_exported: true,
            complexity: Some(12),
        }],
        edges: vec![],
    };

    db.index_extracted_file("repo_1", "src/server.ts", "hash_s", 1, 500, &extracted)
        .unwrap();

    let tmp = tempdir().expect("temp dir");
    let out_file = tmp.path().join("sub/dir/graph.html");

    let (data, resolved_path) = execute_viz(&db, &out_file, Some("repo_1")).expect("execute viz");

    assert_eq!(data.nodes.len(), 1);
    assert_eq!(data.nodes.first().expect("data first node").name, "Server");
    assert!(out_file.exists());
    assert!(resolved_path.exists());

    let content = fs::read_to_string(&resolved_path).expect("read generated html");
    assert!(content.contains("Server"));
    assert!(content.contains("<!DOCTYPE html>"));
}
