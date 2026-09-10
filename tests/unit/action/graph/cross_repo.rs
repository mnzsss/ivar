#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::graph::{EdgeKind, Provenance, Span, Symbol, SymbolKind};
use crate::store::graph::extractor::ExtractedFile;

#[test]
fn test_http_client_calls_link_to_routes_with_path_parameters() {
    let db = GraphDb::open_in_memory().expect("open_in_memory");
    db.insert_repo("api", "/path/to/api", "main", None)
        .expect("insert api repo");
    db.insert_repo("web", "/path/to/web", "main", None)
        .expect("insert web repo");

    let symbol = |repo: &str, name: &str, kind: SymbolKind, line| Symbol {
        id: None,
        file_id: None,
        repo: repo.to_owned(),
        name: name.to_owned(),
        kind,
        scope: None,
        signature: None,
        docstring: None,
        span: Span::new(line, 1, line + 5, 1),
        is_exported: false,
        complexity: None,
    };
    let route = || SymbolKind::Other("route".to_owned());
    db.index_extracted_file(
        "api",
        "src/routes/projects.ts",
        "h1",
        1,
        10,
        &ExtractedFile {
            symbols: vec![
                symbol("api", "GET /projects", route(), 1),
                symbol("api", "GET /projects/:id", route(), 10),
            ],
            edges: vec![],
        },
    )
    .expect("index api");

    let request = |to_name: &str, line| crate::domain::graph::Edge {
        id: None,
        repo: "web".to_owned(),
        file_id: None,
        from_symbol_id: None,
        to_symbol_id: None,
        to_name: Some(to_name.to_owned()),
        kind: EdgeKind::CrossCallsHttp,
        provenance: Provenance::Inferred,
        line,
        col: 10,
        confidence: 0.8,
    };
    db.index_extracted_file(
        "web",
        "src/api/projects.ts",
        "h2",
        1,
        10,
        &ExtractedFile {
            symbols: vec![symbol("web", "projectsApi", SymbolKind::Fn, 1)],
            edges: vec![
                request("GET /projects/:param", 3),
                request("GET /projects", 5),
            ],
        },
    )
    .expect("index web");

    link_cross_repo_edges(&db).expect("link");

    let linked: Vec<(String, String)> = db
        .conn()
        .prepare(
            "SELECT e.to_name, s.name FROM edges e
             JOIN symbols s ON s.id = e.to_symbol_id
             WHERE e.repo = 'web'
             ORDER BY e.line",
        )
        .expect("prepare")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    assert_eq!(
        linked,
        vec![
            (
                "GET /projects/:param".to_owned(),
                "GET /projects/:id".to_owned()
            ),
            ("GET /projects".to_owned(), "GET /projects".to_owned()),
        ]
    );
}

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
            repo: "backend".to_owned(),
            name: "process_payment".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("pub fn process_payment()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: None,
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
            repo: "web".to_owned(),
            name: "checkout".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("export function checkout()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 10, 1),
            is_exported: true,
            complexity: None,
        }],
        edges: vec![crate::domain::graph::Edge {
            id: None,
            repo: "web".to_owned(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("process_payment".to_owned()),
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
            repo: "ivar-cli".to_owned(),
            name: "ivar".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("pub fn main()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 10, 1),
            is_exported: true,
            complexity: None,
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
            repo: "orca-ts".to_owned(),
            name: "spawnIvar".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("function spawnIvar()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: false,
            complexity: None,
        }],
        edges: vec![crate::domain::graph::Edge {
            id: None,
            repo: "orca-ts".to_owned(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("ivar".to_owned()),
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
            repo: "server".to_owned(),
            name: "/api/v1/users".to_owned(),
            kind: SymbolKind::Fn,
            scope: Some("UserController".to_owned()),
            signature: Some("async fn get_users()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 10, 1),
            is_exported: true,
            complexity: None,
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
            repo: "frontend".to_owned(),
            name: "fetchUsers".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("async function fetchUsers()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: false,
            complexity: None,
        }],
        edges: vec![crate::domain::graph::Edge {
            id: None,
            repo: "frontend".to_owned(),
            file_id: None,
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("/api/v1/users".to_owned()),
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
