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
    assert!(!is_test_file("pkg/api/client.go"));
}

#[test]
fn test_parse_files_from_reader() {
    let input = "
        # Comments should be ignored
        src/action/graph/mod.rs
        src/domain/graph.rs

        # Empty lines ignored
        tests/graph_test.rs
    ";
    let files = parse_files_from_reader(input.as_bytes());
    assert_eq!(
        files,
        vec![
            "src/action/graph/mod.rs",
            "src/domain/graph.rs",
            "tests/graph_test.rs"
        ]
    );
}

#[test]
fn test_empty_changed_files_returns_empty_result() -> Result<(), Box<dyn std::error::Error>> {
    let db = GraphDb::open_in_memory()?;
    db.insert_repo("test_repo", "/root", "main", None)?;

    let result = find_affected_tests(&db, &[], Some("test_repo"), 5)?;
    assert!(result.changed_files.is_empty());
    assert!(result.affected_test_files.is_empty());
    assert!(result.recommendations.is_empty());

    let result_blank = find_affected_tests(&db, &["   ".to_owned()], Some("test_repo"), 5)?;
    assert!(result_blank.changed_files.is_empty());
    assert!(result_blank.affected_test_files.is_empty());
    assert!(result_blank.recommendations.is_empty());

    Ok(())
}

#[test]
fn test_direct_test_file_change_selects_itself() -> Result<(), Box<dyn std::error::Error>> {
    let db = GraphDb::open_in_memory()?;
    db.insert_repo("test_repo", "/root", "main", None)?;

    let test_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "test_repo".to_owned(),
            name: "test_something".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn test_something()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: false,
            complexity: None,
        }],
        edges: vec![],
    };

    db.index_extracted_file(
        "test_repo",
        "tests/direct_test.rs",
        "hash_dt",
        100,
        1000,
        &test_extracted,
    )?;

    let result = find_affected_tests(
        &db,
        &["tests/direct_test.rs".to_owned()],
        Some("test_repo"),
        5,
    )?;

    assert_eq!(result.changed_files, vec!["tests/direct_test.rs"]);
    assert_eq!(result.affected_test_files, vec!["tests/direct_test.rs"]);
    assert_eq!(result.recommendations.len(), 1);

    let rec = &result.recommendations[0];
    assert_eq!(rec.test_file, "tests/direct_test.rs");
    assert!(rec.direct_change);
    assert_eq!(rec.hop_count, 0);
    assert_eq!(rec.reason, "direct change to test file");
    assert_eq!(
        rec.command.as_deref(),
        Some("cargo test --test direct_test")
    );

    Ok(())
}

#[test]
fn test_direct_test_consumer_one_hop_explanation() -> Result<(), Box<dyn std::error::Error>> {
    let db = GraphDb::open_in_memory()?;
    db.insert_repo("test_repo", "/root", "main", None)?;

    let src_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "test_repo".to_owned(),
            name: "core_func".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn core_func()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: None,
        }],
        edges: vec![],
    };

    db.index_extracted_file(
        "test_repo",
        "src/core.rs",
        "hash_c",
        100,
        1000,
        &src_extracted,
    )?;

    let test_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "test_repo".to_owned(),
            name: "test_core".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn test_core()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 8, 1),
            is_exported: false,
            complexity: None,
        }],
        edges: vec![Edge {
            id: None,
            repo: "test_repo".to_owned(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("core_func".to_owned()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 4,
            col: 5,
            confidence: 1.0,
        }],
    };

    db.index_extracted_file(
        "test_repo",
        "tests/core_test.rs",
        "hash_ct",
        101,
        1000,
        &test_extracted,
    )?;

    let result = find_affected_tests(&db, &["src/core.rs".to_owned()], Some("test_repo"), 5)?;

    assert_eq!(result.changed_files, vec!["src/core.rs"]);
    assert_eq!(result.affected_test_files, vec!["tests/core_test.rs"]);
    assert_eq!(result.recommendations.len(), 1);

    let rec = &result.recommendations[0];
    assert_eq!(rec.test_file, "tests/core_test.rs");
    assert!(!rec.direct_change);
    assert_eq!(rec.hop_count, 1);
    assert_eq!(rec.edge_kind, EdgeKind::Calls);
    assert_eq!(rec.confidence, 1.0);
    assert_eq!(rec.reason, "calls src/core.rs (1 hop)");
    assert_eq!(rec.command.as_deref(), Some("cargo test --test core_test"));
    assert_eq!(rec.causal_path.len(), 1);
    assert_eq!(rec.causal_path[0].source, "tests/core_test.rs:test_core");
    assert_eq!(rec.causal_path[0].target, "src/core.rs:core_func");

    Ok(())
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
            repo: "test_repo".to_owned(),
            name: "helper_fn".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn helper_fn()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: None,
        }],
        edges: vec![],
    };

    db.index_extracted_file(
        "test_repo",
        "src/utils.rs",
        "hash_u",
        100,
        1000,
        &utils_extracted,
    )?;

    // 2. src/core.rs defines `core_work` and calls `helper_fn`
    let core_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "test_repo".to_owned(),
            name: "core_work".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn core_work()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 10, 1),
            is_exported: true,
            complexity: None,
        }],
        edges: vec![Edge {
            id: None,
            repo: "test_repo".to_owned(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("helper_fn".to_owned()),
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
        "hash_c",
        101,
        1000,
        &core_extracted,
    )?;

    // 3. tests/core_test.rs defines `test_core_feature` and calls `core_work`
    let test_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "test_repo".to_owned(),
            name: "test_core_feature".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn test_core_feature()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 8, 1),
            is_exported: false,
            complexity: None,
        }],
        edges: vec![Edge {
            id: None,
            repo: "test_repo".to_owned(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("core_work".to_owned()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 4,
            col: 4,
            confidence: 0.9,
        }],
    };

    db.index_extracted_file(
        "test_repo",
        "tests/core_test.rs",
        "hash_t",
        102,
        1000,
        &test_extracted,
    )?;

    // Query affected tests for src/utils.rs
    let result = find_affected_tests(&db, &["src/utils.rs".to_owned()], Some("test_repo"), 5)?;

    assert_eq!(result.changed_files, vec!["src/utils.rs"]);
    assert_eq!(result.affected_test_files, vec!["tests/core_test.rs"]);
    assert_eq!(result.recommendations.len(), 1);

    let rec = &result.recommendations[0];
    assert_eq!(rec.test_file, "tests/core_test.rs");
    assert_eq!(rec.hop_count, 2);
    assert_eq!(rec.confidence, 0.9);
    assert_eq!(
        rec.reason,
        "transitively depends on src/utils.rs (2 hops via src/core.rs)"
    );
    assert_eq!(rec.command.as_deref(), Some("cargo test --test core_test"));
    assert_eq!(rec.causal_path.len(), 2);
    assert_eq!(rec.causal_path[0].source, "src/core.rs:core_work");
    assert_eq!(rec.causal_path[0].target, "src/utils.rs:helper_fn");
    assert_eq!(
        rec.causal_path[1].source,
        "tests/core_test.rs:test_core_feature"
    );
    assert_eq!(rec.causal_path[1].target, "src/core.rs:core_work");

    Ok(())
}

#[test]
fn test_cross_repo_test_consumer() -> Result<(), Box<dyn std::error::Error>> {
    let db = GraphDb::open_in_memory()?;
    db.insert_repo("backend_repo", "/backend", "main", None)?;
    db.insert_repo("frontend_repo", "/frontend", "main", None)?;

    let lib_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "backend_repo".to_owned(),
            name: "UserApi".to_owned(),
            kind: SymbolKind::Struct,
            scope: None,
            signature: Some("pub struct UserApi".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: None,
        }],
        edges: vec![],
    };

    db.index_extracted_file(
        "backend_repo",
        "src/api.rs",
        "hash_b",
        100,
        1000,
        &lib_extracted,
    )?;

    let front_test_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "frontend_repo".to_owned(),
            name: "test_api_client".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("function test_api_client()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 10, 1),
            is_exported: false,
            complexity: None,
        }],
        edges: vec![Edge {
            id: None,
            repo: "frontend_repo".to_owned(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("UserApi".to_owned()),
            kind: EdgeKind::CrossImports,
            provenance: Provenance::Inferred,
            line: 2,
            col: 1,
            confidence: 0.8,
        }],
    };

    db.index_extracted_file(
        "frontend_repo",
        "tests/api.test.ts",
        "hash_ft",
        101,
        1000,
        &front_test_extracted,
    )?;

    let result = find_affected_tests(
        &db,
        &["src/api.rs".to_owned()],
        None, // cross repo search
        5,
    )?;

    assert_eq!(result.changed_files, vec!["src/api.rs"]);
    assert_eq!(result.affected_test_files, vec!["tests/api.test.ts"]);
    assert_eq!(result.recommendations.len(), 1);

    let rec = &result.recommendations[0];
    assert_eq!(rec.repo, "frontend_repo");
    assert_eq!(rec.test_file, "tests/api.test.ts");
    assert_eq!(rec.hop_count, 1);
    assert_eq!(rec.edge_kind, EdgeKind::CrossImports);
    assert_eq!(rec.provenance, Provenance::Inferred);
    assert_eq!(rec.confidence, 0.8);

    Ok(())
}

#[test]
fn test_unrecognized_runner_omits_command() {
    let cmd = derive_test_command(None, "custom_repo", "scripts/test_runner.unknown");
    assert!(cmd.is_none());
}

#[test]
fn test_affected_tests_multi_repo_identical_paths() -> Result<(), Box<dyn std::error::Error>> {
    let db = GraphDb::open_in_memory()?;
    db.insert_repo("repo_alpha", "/alpha", "main", None)?;
    db.insert_repo("repo_beta", "/beta", "main", None)?;

    let alpha_src = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "repo_alpha".to_owned(),
            name: "fn_alpha".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn fn_alpha()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: None,
        }],
        edges: vec![],
    };
    db.index_extracted_file("repo_alpha", "src/a.rs", "hash_a", 100, 1000, &alpha_src)?;

    let alpha_test = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "repo_alpha".to_owned(),
            name: "test_alpha".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn test_alpha()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 8, 1),
            is_exported: false,
            complexity: None,
        }],
        edges: vec![Edge {
            id: None,
            repo: "repo_alpha".to_owned(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("fn_alpha".to_owned()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 3,
            col: 5,
            confidence: 1.0,
        }],
    };
    db.index_extracted_file(
        "repo_alpha",
        "tests/unit/foo_test.rs",
        "hash_at",
        101,
        1000,
        &alpha_test,
    )?;

    let beta_src = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "repo_beta".to_owned(),
            name: "fn_beta".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn fn_beta()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: None,
        }],
        edges: vec![],
    };
    db.index_extracted_file("repo_beta", "src/b.rs", "hash_b", 200, 1000, &beta_src)?;

    let beta_test = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "repo_beta".to_owned(),
            name: "test_beta".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn test_beta()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 8, 1),
            is_exported: false,
            complexity: None,
        }],
        edges: vec![Edge {
            id: None,
            repo: "repo_beta".to_owned(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("fn_beta".to_owned()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 4,
            col: 5,
            confidence: 1.0,
        }],
    };
    db.index_extracted_file(
        "repo_beta",
        "tests/unit/foo_test.rs",
        "hash_bt",
        201,
        1000,
        &beta_test,
    )?;

    let result = find_affected_tests(
        &db,
        &["src/a.rs".to_owned(), "src/b.rs".to_owned()],
        None,
        5,
    )?;

    assert_eq!(result.changed_files.len(), 2);
    assert_eq!(result.affected_test_files, vec!["tests/unit/foo_test.rs"]);
    assert_eq!(
        result.recommendations.len(),
        2,
        "Both repos must be retained in recommendations without one overwriting the other"
    );

    let repos: Vec<&str> = result
        .recommendations
        .iter()
        .map(|r| r.repo.as_str())
        .collect();
    assert!(repos.contains(&"repo_alpha"));
    assert!(repos.contains(&"repo_beta"));

    Ok(())
}
