//! Unit tests for class/struct hierarchy analysis action.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::graph::{Edge, EdgeKind, Provenance, Span, Symbol, SymbolKind};
use crate::store::graph::extractor::ExtractedFile;

#[test]
fn test_execute_hierarchy_bases_and_implementations() {
    let db = GraphDb::open_in_memory().expect("open db");
    db.insert_repo("test_repo", "/path", "main", None).unwrap();

    // 1. Interface/trait `BaseService` in `src/service.rs`
    let base_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "test_repo".to_owned(),
            name: "BaseService".to_owned(),
            kind: SymbolKind::Interface,
            scope: None,
            signature: Some("trait BaseService".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 10, 1),
            is_exported: true,
            complexity: None,
        }],
        edges: vec![],
    };

    db.index_extracted_file(
        "test_repo",
        "src/service.rs",
        "h_base",
        1,
        100,
        &base_extracted,
    )
    .unwrap();

    // 2. Base trait `CoreTrait` in `src/core_trait.rs`
    let core_trait_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "test_repo".to_owned(),
            name: "CoreTrait".to_owned(),
            kind: SymbolKind::Interface,
            scope: None,
            signature: Some("trait CoreTrait".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: None,
        }],
        edges: vec![],
    };

    db.index_extracted_file(
        "test_repo",
        "src/core_trait.rs",
        "h_core",
        1,
        100,
        &core_trait_extracted,
    )
    .unwrap();

    // 3. Struct `AuthService` in `src/auth.rs` implementing `BaseService` and inheriting `CoreTrait`
    let auth_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "test_repo".to_owned(),
            name: "AuthService".to_owned(),
            kind: SymbolKind::Struct,
            scope: None,
            signature: Some("struct AuthService".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 30, 1),
            is_exported: true,
            complexity: None,
        }],
        edges: vec![
            Edge {
                id: None,
                repo: "test_repo".to_owned(),
                file_id: None,
                from_symbol_id: None,
                to_symbol_id: None,
                to_name: Some("BaseService".to_owned()),
                kind: EdgeKind::Implements,
                provenance: Provenance::Extracted,
                line: 1,
                col: 1,
                confidence: 1.0,
            },
            Edge {
                id: None,
                repo: "test_repo".to_owned(),
                file_id: None,
                from_symbol_id: None,
                to_symbol_id: None,
                to_name: Some("CoreTrait".to_owned()),
                kind: EdgeKind::Inherits,
                provenance: Provenance::Extracted,
                line: 1,
                col: 1,
                confidence: 1.0,
            },
        ],
    };

    db.index_extracted_file(
        "test_repo",
        "src/auth.rs",
        "h_auth",
        1,
        100,
        &auth_extracted,
    )
    .unwrap();

    // 4. Struct `CustomAuthService` inheriting from `AuthService` in `src/custom_auth.rs`
    let custom_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "test_repo".to_owned(),
            name: "CustomAuthService".to_owned(),
            kind: SymbolKind::Struct,
            scope: None,
            signature: Some("struct CustomAuthService".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 20, 1),
            is_exported: true,
            complexity: None,
        }],
        edges: vec![Edge {
            id: None,
            repo: "test_repo".to_owned(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("AuthService".to_owned()),
            kind: EdgeKind::Inherits,
            provenance: Provenance::Extracted,
            line: 1,
            col: 1,
            confidence: 1.0,
        }],
    };

    db.index_extracted_file(
        "test_repo",
        "src/custom_auth.rs",
        "h_custom",
        1,
        100,
        &custom_extracted,
    )
    .unwrap();

    // Query hierarchy for `AuthService`
    let hierarchy = execute_hierarchy(&db, "AuthService", Some("test_repo"))
        .expect("execute_hierarchy")
        .expect("found");

    assert_eq!(hierarchy.symbol.name, "AuthService");
    assert_eq!(hierarchy.file_path, "src/auth.rs");
    // Bases: BaseService, CoreTrait
    assert!(hierarchy.bases.contains(&"BaseService".to_owned()));
    assert!(hierarchy.bases.contains(&"CoreTrait".to_owned()));
    // Implementations/Subtypes: CustomAuthService
    assert_eq!(
        hierarchy.implementations,
        vec!["CustomAuthService".to_owned()]
    );

    // Query hierarchy for `BaseService`
    let base_hierarchy = execute_hierarchy(&db, "BaseService", Some("test_repo"))
        .expect("execute_hierarchy")
        .expect("found");
    assert_eq!(base_hierarchy.symbol.name, "BaseService");
    assert!(base_hierarchy.bases.is_empty());
    assert_eq!(
        base_hierarchy.implementations,
        vec!["AuthService".to_owned()]
    );

    // Query non-existent symbol
    let not_found =
        execute_hierarchy(&db, "NonExistent", Some("test_repo")).expect("execute_hierarchy");
    assert!(not_found.is_none());
}
