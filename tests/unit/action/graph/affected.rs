#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::graph::{Edge, EdgeKind, Provenance, Span, Symbol, SymbolKind};
use crate::infra::graph::extractor::ExtractedFile;

#[test]
fn test_is_test_file_heuristics() {
    assert!(is_test_file("tests/core_test.rs"));
    assert!(is_test_file("tests/unit/some_test.rs"));
    assert!(is_test_file("src/foo_test.rs"));
    assert!(is_test_file("src/test_bar.rs"));
    assert!(is_test_file("src/lib_test.rs"));
    assert!(is_test_file("tests/integration.rs"));
    assert!(is_test_file("frontend/__tests__/App.test.tsx"));
    assert!(is_test_file("src/components/Button.spec.ts"));
    assert!(is_test_file("src/components/Modal.test.js"));

    assert!(!is_test_file("src/main.rs"));
    assert!(!is_test_file("src/utils.rs"));
    assert!(!is_test_file("src/core.rs"));
}

#[test]
fn test_parse_files_from_reader() {
    let input = "src/foo.rs\n\n# A comment\n  src/bar.rs  \n";
    let files = parse_files_from_reader(input.as_bytes());
    assert_eq!(files, vec!["src/foo.rs", "src/bar.rs"]);
}

#[test]
fn test_find_affected_tests_transitive() -> Result<(), Box<dyn std::error::Error>> {
    let db = GraphDb::open_in_memory()?;
    db.insert_repo("test_repo", "/root", "main", None)?;

    // 1. src/utils.rs defines `helper_fn`
    let utils_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "test_repo".to_string(),
            name: "helper_fn".to_string(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn helper_fn()".to_string()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
        }],
        edges: vec![],
    };
    db.index_extracted_file(
        "test_repo",
        "src/utils.rs",
        "hash_utils",
        100,
        100,
        &utils_extracted,
    )?;

    // 2. src/core.rs defines `core_work` and calls `helper_fn`
    let core_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "test_repo".to_string(),
            name: "core_work".to_string(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn core_work()".to_string()),
            docstring: None,
            span: Span::new(1, 1, 10, 1),
            is_exported: true,
        }],
        edges: vec![Edge {
            id: None,
            repo: "test_repo".to_string(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("helper_fn".to_string()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 5,
            col: 4,
            confidence: 1.0,
        }],
    };
    db.index_extracted_file(
        "test_repo",
        "src/core.rs",
        "hash_core",
        200,
        200,
        &core_extracted,
    )?;

    // 3. tests/core_test.rs defines `test_core_feature` and calls `core_work`
    let test_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "test_repo".to_string(),
            name: "test_core_feature".to_string(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn test_core_feature()".to_string()),
            docstring: None,
            span: Span::new(1, 1, 8, 1),
            is_exported: false,
        }],
        edges: vec![Edge {
            id: None,
            repo: "test_repo".to_string(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("core_work".to_string()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 4,
            col: 4,
            confidence: 1.0,
        }],
    };
    db.index_extracted_file(
        "test_repo",
        "tests/core_test.rs",
        "hash_test",
        300,
        300,
        &test_extracted,
    )?;

    // Query affected tests for src/utils.rs
    let result = find_affected_tests(
        &db,
        &["src/utils.rs".to_string()],
        Some("test_repo"),
        5,
    )?;

    assert_eq!(result.changed_files, vec!["src/utils.rs"]);
    assert_eq!(result.affected_test_files, vec!["tests/core_test.rs"]);

    Ok(())
}
