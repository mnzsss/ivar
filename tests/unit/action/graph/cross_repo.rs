#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::graph::{EdgeKind, Provenance, Span, Symbol, SymbolKind};
use crate::infra::graph::extractor::ExtractedFile;

#[test]
fn test_cross_repo_import_linking() {
    let db = GraphDb::open_in_memory().expect("open_in_memory");

    // Repo B ("backend"): exports `process_payment`
    db.insert_repo("backend", "/path/to/backend", "main", None)
        .expect("insert repo");
    let backend_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "backend".to_string(),
            name: "process_payment".to_string(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("pub fn process_payment()".to_string()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
        }],
        edges: vec![],
    };
    db.index_extracted_file(
        "backend",
        "src/payment.rs",
        "hash_b1",
        100,
        500,
        &backend_extracted,
    )
    .expect("index backend");

    // Repo A ("web"): calls `process_payment`, currently dangling
    db.insert_repo("web", "/path/to/web", "main", None)
        .expect("insert repo");
    let web_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "web".to_string(),
            name: "checkout".to_string(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("export function checkout()".to_string()),
            docstring: None,
            span: Span::new(1, 1, 10, 1),
            is_exported: true,
        }],
        edges: vec![crate::domain::graph::Edge {
            id: None,
            repo: "web".to_string(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("process_payment".to_string()),
            kind: EdgeKind::Imports,
            provenance: Provenance::Extracted,
            line: 2,
            col: 1,
            confidence: 0.95,
        }],
    };
    db.index_extracted_file(
        "web",
        "src/checkout.ts",
        "hash_w1",
        100,
        300,
        &web_extracted,
    )
    .expect("index web");

    // Run cross repo linking
    let outcome = link_cross_repo_edges(&db).expect("link_cross_repo_edges");
    assert_eq!(outcome.cross_imports, 1);
    assert_eq!(outcome.total_linked, 1);

    let backend_sym_id: i64 = db
        .conn()
        .query_row(
            "SELECT id FROM symbols WHERE repo = 'backend' AND name = 'process_payment'",
            [],
            |row| row.get(0),
        )
        .expect("backend symbol exists");

    let edges = db
        .conn()
        .prepare("SELECT to_symbol_id, kind, provenance, confidence FROM edges WHERE repo = 'web'")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<i64>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect::<Vec<_>>();

    assert_eq!(edges.len(), 1);
    let (to_sym_id, kind, prov, conf) = &edges[0];
    assert_eq!(*to_sym_id, Some(backend_sym_id));
    assert_eq!(kind, "CROSS_IMPORTS");
    assert_eq!(prov, "INFERRED");
    assert!((conf - 0.90).abs() < f64::EPSILON);
}

#[test]
fn test_cross_repo_cli_execution_linking() {
    let db = GraphDb::open_in_memory().expect("open_in_memory");

    // Repo B ("ivar-cli"): defines binary / CLI command entry `ivar`
    db.insert_repo("ivar-cli", "/path/to/cli", "main", None)
        .expect("insert repo");
    let cli_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "ivar-cli".to_string(),
            name: "ivar".to_string(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("pub fn main()".to_string()),
            docstring: None,
            span: Span::new(1, 1, 10, 1),
            is_exported: true,
        }],
        edges: vec![],
    };
    db.index_extracted_file(
        "ivar-cli",
        "src/main.rs",
        "hash_cli1",
        100,
        400,
        &cli_extracted,
    )
    .expect("index cli");

    // Repo A ("orca-ts"): executes CLI binary `ivar`
    db.insert_repo("orca-ts", "/path/to/orca", "main", None)
        .expect("insert repo");
    let orca_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "orca-ts".to_string(),
            name: "spawnIvar".to_string(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("function spawnIvar()".to_string()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: false,
        }],
        edges: vec![crate::domain::graph::Edge {
            id: None,
            repo: "orca-ts".to_string(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("ivar".to_string()),
            kind: EdgeKind::CrossExecutes,
            provenance: Provenance::Inferred,
            line: 3,
            col: 5,
            confidence: 0.85,
        }],
    };
    db.index_extracted_file(
        "orca-ts",
        "src/runner.ts",
        "hash_orca1",
        100,
        350,
        &orca_extracted,
    )
    .expect("index orca");

    let outcome = link_cross_repo_edges(&db).expect("link cross repo");
    assert_eq!(outcome.cross_executes, 1);
    assert_eq!(outcome.total_linked, 1);

    let cli_sym_id: i64 = db
        .conn()
        .query_row(
            "SELECT id FROM symbols WHERE repo = 'ivar-cli' AND name = 'ivar'",
            [],
            |row| row.get(0),
        )
        .expect("cli symbol exists");

    let to_sym_id: Option<i64> = db
        .conn()
        .query_row(
            "SELECT to_symbol_id FROM edges WHERE repo = 'orca-ts'",
            [],
            |row| row.get(0),
        )
        .expect("query edge");
    assert_eq!(to_sym_id, Some(cli_sym_id));
}

#[test]
fn test_cross_repo_http_linking() {
    let db = GraphDb::open_in_memory().expect("open_in_memory");

    // Repo B ("server"): defines route handler symbol `/api/v1/users`
    db.insert_repo("server", "/path/to/server", "main", None)
        .expect("insert repo");
    let server_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "server".to_string(),
            name: "/api/v1/users".to_string(),
            kind: SymbolKind::Fn,
            scope: Some("UserController".to_string()),
            signature: Some("async fn get_users()".to_string()),
            docstring: None,
            span: Span::new(1, 1, 10, 1),
            is_exported: true,
        }],
        edges: vec![],
    };
    db.index_extracted_file(
        "server",
        "src/routes.rs",
        "hash_srv1",
        100,
        400,
        &server_extracted,
    )
    .expect("index server");

    // Repo A ("frontend"): calls endpoint `/api/v1/users`
    db.insert_repo("frontend", "/path/to/client", "main", None)
        .expect("insert repo");
    let client_extracted = ExtractedFile {
        symbols: vec![Symbol {
            id: None,
            file_id: None,
            repo: "frontend".to_string(),
            name: "fetchUsers".to_string(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("async function fetchUsers()".to_string()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: false,
        }],
        edges: vec![crate::domain::graph::Edge {
            id: None,
            repo: "frontend".to_string(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("/api/v1/users".to_string()),
            kind: EdgeKind::CrossCallsHttp,
            provenance: Provenance::Inferred,
            line: 2,
            col: 5,
            confidence: 0.80,
        }],
    };
    db.index_extracted_file(
        "frontend",
        "src/api.ts",
        "hash_fe1",
        100,
        350,
        &client_extracted,
    )
    .expect("index client");

    let outcome = link_cross_repo_edges(&db).expect("link cross repo");
    assert_eq!(outcome.cross_calls_http, 1);
    assert_eq!(outcome.total_linked, 1);

    let srv_sym_id: i64 = db
        .conn()
        .query_row(
            "SELECT id FROM symbols WHERE repo = 'server' AND name = '/api/v1/users'",
            [],
            |row| row.get(0),
        )
        .expect("server symbol exists");

    let to_sym_id: Option<i64> = db
        .conn()
        .query_row(
            "SELECT to_symbol_id FROM edges WHERE repo = 'frontend'",
            [],
            |row| row.get(0),
        )
        .expect("query edge");
    assert_eq!(to_sym_id, Some(srv_sym_id));
}
