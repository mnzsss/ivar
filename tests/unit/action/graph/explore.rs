#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use std::fs;
use tempfile::tempdir;

use crate::domain::graph::{EdgeKind, Provenance, Span, Symbol, SymbolKind};

#[test]
fn test_explore_hero_query() {
    let temp = tempdir().expect("create temp dir");
    let hall_root = temp.path();

    // Create mock repository directory and source file
    let repo_dir = hall_root.join("test-repo");
    fs::create_dir_all(repo_dir.join("src")).expect("create src dir");
    let file_rel_path = "src/hall.rs";
    let file_content = r#"// Header comment
pub fn init_hall(config: Config) -> Result<Hall> {
    let hall = Hall::new(config);
    setup_logging(&hall);
    Ok(hall)
}

pub fn caller_func() {
    init_hall(Config::default());
}
"#;
    fs::write(repo_dir.join(file_rel_path), file_content).expect("write source file");

    let db = GraphDb::open_in_memory().expect("open memory db");
    db.insert_repo(
        "test-repo",
        repo_dir.to_str().unwrap(),
        "main",
        Some("commit1"),
    )
    .expect("insert repo");

    let file_id = db
        .upsert_file(
            "test-repo",
            file_rel_path,
            "hash1",
            1000,
            file_content.len() as i64,
        )
        .expect("upsert file");

    let init_hall_sym = Symbol {
        id: None,
        file_id: Some(file_id),
        repo: "test-repo".to_owned(),
        name: "init_hall".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("pub fn init_hall(config: Config) -> Result<Hall>".to_owned()),
        docstring: None,
        span: Span::new(2, 1, 6, 2),
        is_exported: true,
        complexity: None,
    };

    let caller_sym = Symbol {
        id: None,
        file_id: Some(file_id),
        repo: "test-repo".to_owned(),
        name: "caller_func".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("pub fn caller_func()".to_owned()),
        docstring: None,
        span: Span::new(8, 1, 10, 2),
        is_exported: true,
        complexity: None,
    };

    let sym_ids = db
        .insert_symbols(&[init_hall_sym, caller_sym])
        .expect("insert symbols");
    let init_hall_id = sym_ids[0];
    let caller_func_id = sym_ids[1];

    // Add edge: caller_func -> init_hall (CALLS)
    let edge1 = crate::domain::graph::Edge {
        id: None,
        file_id: Some(file_id),
        repo: "test-repo".to_owned(),
        from_symbol_id: Some(caller_func_id),
        to_symbol_id: Some(init_hall_id),
        to_name: Some("init_hall".to_owned()),
        kind: EdgeKind::Calls,
        provenance: Provenance::Extracted,
        line: 9,
        col: 5,
        confidence: 1.0,
    };

    // Add edge: init_hall -> setup_logging (CALLS, unresolved)
    let edge2 = crate::domain::graph::Edge {
        id: None,
        file_id: Some(file_id),
        repo: "test-repo".to_owned(),
        from_symbol_id: Some(init_hall_id),
        to_symbol_id: None,
        to_name: Some("setup_logging".to_owned()),
        kind: EdgeKind::Calls,
        provenance: Provenance::Extracted,
        line: 4,
        col: 5,
        confidence: 1.0,
    };

    db.insert_edges(&[edge1, edge2]).expect("insert edges");

    // Run explore
    let result =
        explore(&db, hall_root, "init_hall", Some("test-repo")).expect("explore should succeed");

    assert_eq!(result.query, "init_hall");
    assert_eq!(result.primary_symbols.len(), 1);

    let snippet = &result.primary_symbols[0];
    assert_eq!(snippet.symbol.name, "init_hall");
    assert_eq!(snippet.file_path, "src/hall.rs");
    assert_eq!(snippet.start_line, 2);
    assert_eq!(snippet.end_line, 6);

    // Verbatim code lines check
    assert!(
        snippet
            .code
            .contains("2\tpub fn init_hall(config: Config) -> Result<Hall> {")
    );
    assert!(snippet.code.contains("4\t    setup_logging(&hall);"));
    assert!(snippet.code.contains("6\t}"));

    // Call flows check
    assert_eq!(result.call_flows.len(), 2);
    let caller_flow = result
        .call_flows
        .iter()
        .find(|f| f.caller == "caller_func")
        .unwrap();
    assert_eq!(caller_flow.callee, "init_hall");
    assert_eq!(caller_flow.line, 9);

    let callee_flow = result
        .call_flows
        .iter()
        .find(|f| f.callee == "setup_logging")
        .unwrap();
    assert_eq!(callee_flow.caller, "init_hall");
    assert_eq!(callee_flow.line, 4);

    // Impact summary check
    assert!(result.impact_summary.is_some());
    let summary = result.impact_summary.unwrap();
    assert!(summary.contains("Modifying 'init_hall' directly impacts 1 caller across 1 file."));

    // Direct relations & entry points check
    assert_eq!(result.direct_relations.len(), 2);
    let in_rel = result
        .direct_relations
        .iter()
        .find(|r| r.source.symbol_name == "caller_func")
        .unwrap();
    assert_eq!(in_rel.target.symbol_name, "init_hall");
    assert_eq!(in_rel.source.repo, "test-repo");
    assert_eq!(in_rel.source.file_path, "src/hall.rs");
    assert_eq!(
        in_rel.direction,
        crate::domain::graph::RelationDirection::Incoming
    );
    assert_eq!(in_rel.edge_kind, EdgeKind::Calls);
    assert_eq!(in_rel.provenance, Provenance::Extracted);
    assert_eq!(in_rel.confidence, 1.0);
    assert_eq!(in_rel.hop_count, 1);
    assert!(!in_rel.cross_repo);

    // Entry points check (caller_func is exported)
    assert_eq!(result.entry_points.len(), 1);
    assert_eq!(result.entry_points[0].source.symbol_name, "caller_func");

    // Transitive consumers check
    assert_eq!(result.transitive_consumers.len(), 1);
    assert_eq!(result.transitive_consumers[0].symbol_name, "caller_func");
    assert_eq!(result.transitive_consumers[0].depth, 1);
    assert_eq!(result.transitive_consumers[0].repo, "test-repo");
    assert_eq!(result.transitive_consumers[0].file_path, "src/hall.rs");
}

#[test]
fn test_explore_cross_repo_and_inferred_relations() {
    let temp = tempdir().expect("create temp dir");
    let hall_root = temp.path();

    let repo1_dir = hall_root.join("ivar");
    fs::create_dir_all(repo1_dir.join("src/action/feature")).expect("create dir");
    let create_content = "pub fn create() -> Result<()> { Ok(()) }";
    fs::write(
        repo1_dir.join("src/action/feature/create.rs"),
        create_content,
    )
    .expect("write create.rs");

    let repo2_dir = hall_root.join("ivar-orca");
    fs::create_dir_all(repo2_dir.join("src")).expect("create orca dir");
    let orca_content = "export function runCreate() { create(); }";
    fs::write(repo2_dir.join("src/bridge.ts"), orca_content).expect("write bridge.ts");

    let db = GraphDb::open_in_memory().expect("open memory db");
    db.insert_repo("ivar", repo1_dir.to_str().unwrap(), "main", None)
        .expect("insert repo1");
    db.insert_repo("ivar-orca", repo2_dir.to_str().unwrap(), "main", None)
        .expect("insert repo2");

    let file_id1 = db
        .upsert_file(
            "ivar",
            "src/action/feature/create.rs",
            "h1",
            100,
            create_content.len() as i64,
        )
        .expect("f1");
    let file_id2 = db
        .upsert_file(
            "ivar-orca",
            "src/bridge.ts",
            "h2",
            100,
            orca_content.len() as i64,
        )
        .expect("f2");

    let create_sym = Symbol {
        id: None,
        file_id: Some(file_id1),
        repo: "ivar".to_owned(),
        name: "create".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("pub fn create() -> Result<()>".to_owned()),
        docstring: None,
        span: Span::new(1, 1, 1, 40),
        is_exported: true,
        complexity: None,
    };

    let orca_sym = Symbol {
        id: None,
        file_id: Some(file_id2),
        repo: "ivar-orca".to_owned(),
        name: "runCreate".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("export function runCreate()".to_owned()),
        docstring: None,
        span: Span::new(1, 1, 1, 40),
        is_exported: true,
        complexity: None,
    };

    let sym_ids = db
        .insert_symbols(&[create_sym, orca_sym])
        .expect("insert syms");
    let create_id = sym_ids[0];
    let orca_id = sym_ids[1];

    // Cross-repo inferred call edge
    let edge = crate::domain::graph::Edge {
        id: None,
        file_id: Some(file_id2),
        repo: "ivar-orca".to_owned(),
        from_symbol_id: Some(orca_id),
        to_symbol_id: Some(create_id),
        to_name: Some("create".to_owned()),
        kind: EdgeKind::Calls,
        provenance: Provenance::Inferred,
        line: 1,
        col: 31,
        confidence: 0.85,
    };
    db.insert_edges(&[edge]).expect("insert edges");

    let result = explore(&db, hall_root, "create", Some("ivar")).expect("explore");
    assert_eq!(result.primary_symbols.len(), 1);
    assert_eq!(result.primary_symbols[0].symbol.name, "create");
    assert_eq!(result.primary_symbols[0].symbol.repo, "ivar");

    // Direct cross-repo relation
    assert_eq!(result.direct_relations.len(), 1);
    let rel = &result.direct_relations[0];
    assert_eq!(rel.source.repo, "ivar-orca");
    assert_eq!(rel.source.file_path, "src/bridge.ts");
    assert_eq!(rel.source.symbol_name, "runCreate");
    assert_eq!(rel.target.repo, "ivar");
    assert_eq!(rel.target.symbol_name, "create");
    assert_eq!(rel.provenance, Provenance::Inferred);
    assert_eq!(rel.confidence, 0.85);
    assert!(rel.cross_repo);

    // Transitive consumers across repo
    assert_eq!(result.transitive_consumers.len(), 1);
    assert_eq!(result.transitive_consumers[0].repo, "ivar-orca");
    assert_eq!(result.transitive_consumers[0].symbol_name, "runCreate");
    assert!(result.transitive_consumers[0].cross_repo);
}
#[test]
fn test_explore_empty_or_not_found() {
    let temp = tempdir().expect("create temp dir");
    let db = GraphDb::open_in_memory().expect("open memory db");

    let empty_res = explore(&db, temp.path(), "   ", None).expect("empty query");
    assert!(empty_res.primary_symbols.is_empty());
    assert!(empty_res.call_flows.is_empty());

    let not_found_res = explore(&db, temp.path(), "non_existent_func", None).expect("not found");
    assert!(not_found_res.primary_symbols.is_empty());
    assert!(
        not_found_res
            .impact_summary
            .unwrap()
            .contains("No symbols found")
    );
}

#[test]
fn a_file_path_query_resolves_to_that_file_symbols() {
    // Agents address `explore` with paths far more often than with bare names,
    // and they write the path they see in their workspace
    // (`services/api/src/routes/auth.ts`) while the graph stores it relative to
    // its repo (`src/routes/auth.ts`). Neither is a suffix of the other in a
    // fixed direction, so both are tried.
    let temp = tempdir().expect("create temp dir");
    let hall_root = temp.path();
    let repo_dir = hall_root.join("api");
    fs::create_dir_all(repo_dir.join("src/routes")).expect("create dirs");

    let file_rel_path = "src/routes/auth.ts";
    let file_content = "export const authRoutes = async (fastify) => {};\n";
    fs::write(repo_dir.join(file_rel_path), file_content).expect("write source");

    let db = GraphDb::open_in_memory().expect("open memory db");
    db.insert_repo("api", repo_dir.to_str().unwrap(), "main", None)
        .expect("insert repo");
    let file_id = db
        .upsert_file("api", file_rel_path, "h", 1, file_content.len() as i64)
        .expect("upsert file");
    db.insert_symbols(&[Symbol {
        id: None,
        file_id: Some(file_id),
        repo: "api".to_owned(),
        name: "authRoutes".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: None,
        docstring: None,
        span: Span::new(1, 1, 1, 49),
        is_exported: true,
        complexity: None,
    }])
    .expect("insert symbol");

    for query in [
        "src/routes/auth.ts",
        "services/api/src/routes/auth.ts",
        "./src/routes/auth.ts",
    ] {
        let res = explore(&db, hall_root, query, None).expect("explore");
        assert_eq!(
            res.primary_symbols
                .iter()
                .map(|s| s.symbol.name.as_str())
                .collect::<Vec<_>>(),
            vec!["authRoutes"],
            "query `{query}` must resolve to the file's symbols"
        );
    }
}
#[test]
fn test_explore_routes_aggregate_query() {
    let temp = tempdir().expect("create temp dir");
    let hall_root = temp.path();
    let repo_dir = hall_root.join("api");
    fs::create_dir_all(repo_dir.join("src/routes")).expect("create dirs");

    let file_rel_path = "src/routes/api.ts";
    let file_content = r#"
export async function apiRoutes(fastify) {
  fastify.get('/health', async () => ({ status: 'ok' }));
  fastify.post('/users', async () => ({ id: 1 }));
}
"#;
    fs::write(repo_dir.join(file_rel_path), file_content).expect("write source");

    let db = GraphDb::open_in_memory().expect("open memory db");
    db.insert_repo("api", repo_dir.to_str().unwrap(), "main", None)
        .expect("insert repo");
    let file_id = db
        .upsert_file("api", file_rel_path, "h", 1, file_content.len() as i64)
        .expect("upsert file");

    db.insert_symbols(&[
        Symbol {
            id: None,
            file_id: Some(file_id),
            repo: "api".to_owned(),
            name: "GET /health".to_owned(),
            kind: SymbolKind::Other("route".to_owned()),
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(3, 3, 3, 58),
            is_exported: false,
            complexity: None,
        },
        Symbol {
            id: None,
            file_id: Some(file_id),
            repo: "api".to_owned(),
            name: "POST /users".to_owned(),
            kind: SymbolKind::Other("route".to_owned()),
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(4, 3, 4, 51),
            is_exported: false,
            complexity: None,
        },
    ])
    .expect("insert symbols");

    for query in ["API routes endpoints", "routes", "endpoints"] {
        let res = explore(&db, hall_root, query, None).expect("explore");
        let matched_names: Vec<&str> = res
            .primary_symbols
            .iter()
            .map(|s| s.symbol.name.as_str())
            .collect();
        assert!(
            matched_names.contains(&"GET /health"),
            "query `{query}` should find GET /health, got {matched_names:?}"
        );
        assert!(
            matched_names.contains(&"POST /users"),
            "query `{query}` should find POST /users, got {matched_names:?}"
        );
        // Verify code snippet is present
        let health_sym = res
            .primary_symbols
            .iter()
            .find(|s| s.symbol.name == "GET /health")
            .unwrap();
        assert!(
            health_sym.code.contains("/health"),
            "snippet should include line content"
        );
    }
}

fn fn_symbol(file_id: i64, name: &str, start: usize, end: usize) -> Symbol {
    Symbol {
        id: None,
        file_id: Some(file_id),
        repo: "api".to_owned(),
        name: name.to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: None,
        docstring: None,
        span: Span::new(start, 1, end, 1),
        is_exported: true,
        complexity: None,
    }
}

fn index_single_file(
    content: &str,
    path: &str,
    symbols: &[(&str, usize, usize)],
) -> (GraphDb, tempfile::TempDir) {
    let temp = tempdir().expect("create temp dir");
    let repo_dir = temp.path().join("api");
    fs::create_dir_all(repo_dir.join("src")).expect("create dirs");
    fs::write(repo_dir.join(path), content).expect("write source");

    let db = GraphDb::open_in_memory().expect("open memory db");
    db.insert_repo("api", repo_dir.to_str().unwrap(), "main", None)
        .expect("insert repo");
    let file_id = db
        .upsert_file(
            "api",
            path,
            &crate::infra::hash::text(content),
            1,
            content.len() as i64,
        )
        .expect("upsert file");
    let symbols: Vec<Symbol> = symbols
        .iter()
        .map(|(name, start, end)| fn_symbol(file_id, name, *start, *end))
        .collect();
    db.insert_symbols(&symbols).expect("insert symbols");
    (db, temp)
}

#[test]
fn a_small_file_is_returned_whole_once_even_with_several_matches() {
    let content = "import { db } from './db';\n\nexport function createSession() {\n  return db.insert();\n}\n\nexport function getSession(id) {\n  return db.get(id);\n}\n";
    let (db, temp) = index_single_file(
        content,
        "src/sessions.ts",
        &[("createSession", 3, 5), ("getSession", 7, 9)],
    );

    let res = explore(&db, temp.path(), "src/sessions.ts", None).expect("explore");

    assert_eq!(res.sources.len(), 1);
    let source = &res.sources[0];
    assert_eq!(
        (
            source.repo.as_str(),
            source.file_path.as_str(),
            source.line_count
        ),
        ("api", "src/sessions.ts", 9)
    );
    assert_eq!(source.excerpts.len(), 1);
    assert_eq!(
        (source.excerpts[0].start_line, source.excerpts[0].end_line),
        (1, 9)
    );
    assert!(
        source.excerpts[0]
            .code
            .starts_with("1\timport { db } from './db';"),
        "lines outside every symbol belong to the file too"
    );
}

#[test]
fn a_large_file_is_returned_as_merged_excerpts_around_the_matches() {
    let content: String = (1..=400).map(|n| format!("// line {n}\n")).collect();
    let (db, temp) = index_single_file(
        &content,
        "src/big.ts",
        &[("first", 10, 12), ("second", 18, 20), ("far", 300, 305)],
    );

    let res = explore(&db, temp.path(), "src/big.ts", None).expect("explore");

    assert_eq!(res.sources.len(), 1);
    let source = &res.sources[0];
    assert_eq!(source.line_count, 400);
    let ranges: Vec<(usize, usize)> = source
        .excerpts
        .iter()
        .map(|e| (e.start_line, e.end_line))
        .collect();
    assert_eq!(ranges, vec![(10, 20), (300, 305)]);
    assert!(source.excerpts[0].code.contains("15\t// line 15"));
    assert!(!source.changed_since_index);
}

#[test]
fn a_large_file_changed_since_the_index_is_returned_whole_and_flagged() {
    let indexed: String = (1..=400).map(|n| format!("// line {n}\n")).collect();
    let (db, temp) = index_single_file(
        &indexed,
        "src/big.ts",
        &[("first", 10, 12), ("far", 300, 305)],
    );
    fs::write(
        temp.path().join("api/src/big.ts"),
        format!("// a new first line\n{indexed}"),
    )
    .expect("edit source");

    let res = explore(&db, temp.path(), "src/big.ts", None).expect("explore");

    let source = &res.sources[0];
    assert!(source.changed_since_index);
    let ranges: Vec<(usize, usize)> = source
        .excerpts
        .iter()
        .map(|e| (e.start_line, e.end_line))
        .collect();
    assert_eq!(ranges, vec![(1, 401)]);
}

#[test]
fn symbols_named_together_come_with_the_call_path_between_them() {
    let content: String = (1..=30).map(|n| format!("// line {n}\n")).collect();
    let (db, temp) = index_single_file(
        &content,
        "src/flow.ts",
        &[
            ("handleLogin", 1, 5),
            ("createSession", 10, 15),
            ("saveSession", 20, 25),
        ],
    );
    let file_id = db
        .get_file("api", "src/flow.ts")
        .expect("get file")
        .expect("file row")
        .id;
    let id = |name: &str| {
        crate::action::graph::query::find_symbols(&db, name, None, 1).expect("find symbol")[0]
            .symbol
            .id
            .expect("symbol id")
    };
    let call = |from: &str, to: &str, line| crate::domain::graph::Edge {
        id: None,
        repo: "api".to_owned(),
        file_id: Some(file_id),
        from_symbol_id: Some(id(from)),
        to_symbol_id: Some(id(to)),
        to_name: Some(to.to_owned()),
        kind: EdgeKind::Calls,
        provenance: Provenance::Extracted,
        line,
        col: 3,
        confidence: 1.0,
    };
    db.insert_edges(&[
        call("handleLogin", "createSession", 3),
        call("createSession", "saveSession", 12),
    ])
    .expect("insert edges");

    let res = explore(&db, temp.path(), "handleLogin saveSession", None).expect("explore");

    let hops: Vec<Vec<&str>> = res
        .flows
        .iter()
        .map(|flow| flow.steps.iter().map(|step| step.target.as_str()).collect())
        .collect();
    assert_eq!(hops, vec![vec!["createSession", "saveSession"]]);
}

#[test]
fn same_name_definitions_in_different_repos_keep_their_consumers_apart() {
    let temp = tempdir().expect("create temp dir");
    let hall_root = temp.path();
    let db = GraphDb::open_in_memory().expect("open memory db");

    for repo in ["api", "web"] {
        let repo_dir = hall_root.join(repo);
        fs::create_dir_all(repo_dir.join("src")).expect("create src dir");
        let content = "pub struct Session;\nfn consume() {}\n";
        fs::write(repo_dir.join("src/session.rs"), content).expect("write source");
        db.insert_repo(repo, repo_dir.to_str().unwrap(), "main", None)
            .expect("insert repo");
        let file_id = db
            .upsert_file(repo, "src/session.rs", repo, 100, content.len() as i64)
            .expect("upsert file");
        let mut session = fn_symbol(file_id, "Session", 1, 1);
        session.repo = repo.to_owned();
        session.kind = SymbolKind::Struct;
        let mut consumer = fn_symbol(file_id, &format!("{repo}_consumer"), 2, 2);
        consumer.repo = repo.to_owned();
        let ids = db
            .insert_symbols(&[session, consumer])
            .expect("insert symbols");
        let edge = |from: Option<i64>, kind: EdgeKind, line: usize| crate::domain::graph::Edge {
            id: None,
            file_id: Some(file_id),
            repo: repo.to_owned(),
            from_symbol_id: from,
            to_symbol_id: Some(ids[0]),
            to_name: Some("Session".to_owned()),
            kind,
            provenance: Provenance::Extracted,
            line,
            col: 1,
            confidence: 1.0,
        };
        db.insert_edges(&[
            edge(Some(ids[1]), EdgeKind::Calls, 2),
            edge(None, EdgeKind::References, 1),
        ])
        .expect("insert edges");
    }

    let result = explore(&db, hall_root, "Session", None).expect("explore");

    let api_incoming: Vec<_> = result
        .direct_relations
        .iter()
        .filter(|r| r.target.repo == "api" && r.target.symbol_name == "Session")
        .collect();
    assert!(
        api_incoming.iter().all(|r| r.source.repo == "api"),
        "web consumers leaked into api: {api_incoming:?}"
    );
    assert!(
        api_incoming
            .iter()
            .any(|r| r.source.symbol_name == "api_consumer"),
        "got: {api_incoming:?}"
    );
    assert!(
        api_incoming
            .iter()
            .any(|r| r.edge_kind == EdgeKind::References && r.source.symbol_name.is_empty()),
        "type use missing: {api_incoming:?}"
    );
}

#[test]
fn requested_paths_come_back_whole_beyond_the_intent_budget() {
    let temp = tempdir().expect("create temp dir");
    let repo_dir = temp.path().join("api");
    fs::create_dir_all(repo_dir.join("src")).expect("create dirs");
    let db = GraphDb::open_in_memory().expect("open memory db");
    db.insert_repo("api", repo_dir.to_str().unwrap(), "main", None)
        .expect("insert repo");
    let content: String = (1..=400).map(|n| format!("// line {n}\n")).collect();
    let paths: Vec<String> = (0..8).map(|n| format!("src/file{n}.ts")).collect();
    for (n, path) in paths.iter().enumerate() {
        fs::write(repo_dir.join(path), &content).expect("write source");
        let file_id = db
            .upsert_file("api", path, &crate::infra::hash::text(&content), 1, 1)
            .expect("upsert file");
        db.insert_symbols(&[fn_symbol(file_id, &format!("f{n}"), 10, 12)])
            .expect("insert symbols");
    }

    let res = explore_files(&db, temp.path(), &paths.join(" "), None).expect("explore");

    assert!(res.not_shown.is_empty());
    assert_eq!(res.sources.len(), paths.len());
    for source in &res.sources {
        let ranges: Vec<(usize, usize)> = source
            .excerpts
            .iter()
            .map(|e| (e.start_line, e.end_line))
            .collect();
        assert_eq!(ranges, vec![(1, 400)], "{} is whole", source.file_path);
    }
}

struct ExploreFixture {
    pub(super) db: GraphDb,
    pub(super) temp: tempfile::TempDir,
}

impl ExploreFixture {
    fn root(&self) -> &Path {
        self.temp.path()
    }

    fn index_text(&self, repo: &str, path: &str, content: &str) {
        let repo_dir = self.temp.path().join(repo);
        let file_path = repo_dir.join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).expect("create dir");
        }
        fs::write(&file_path, content).expect("write file");

        let _ = self
            .db
            .insert_repo(repo, repo_dir.to_str().unwrap(), "main", None);
        let hash = crate::infra::hash::text(content);
        let empty_extracted = crate::store::graph::extractor::ExtractedFile::default();
        let _ = self.db.index_extracted_file(
            repo,
            path,
            &hash,
            100,
            content.len() as i64,
            content,
            false,
            &empty_extracted,
        );
    }
}

fn explore_fixture() -> ExploreFixture {
    let temp = tempfile::tempdir().expect("tempdir");
    let db = GraphDb::open_in_memory().expect("open memory db");
    ExploreFixture { db, temp }
}

#[test]
fn text_only_hit_builds_an_explore_result_without_fake_symbols() {
    let fixture = explore_fixture();
    fixture.index_text("app", "Dockerfile", "FROM scratch\n# release-image\n");

    let result = explore(
        &fixture.db,
        fixture.root(),
        "Dockerfile release-image",
        Some("app"),
    )
    .unwrap();

    assert!(result.primary_symbols.is_empty());
    assert_eq!(result.file_matches[0].file_path, "Dockerfile");
    assert!(result.sources[0].excerpts[0].code.contains("release-image"));
}

#[test]
fn test_explore_ranged_path_returns_exact_lines() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let hall_root = temp.path();
    let repo_dir = hall_root.join("api");
    std::fs::create_dir_all(repo_dir.join("src")).expect("create dirs");

    let lines: Vec<String> = (1..=100).map(|i| format!("fn line_{i}() {{}}\n")).collect();
    let content = lines.concat();
    std::fs::write(repo_dir.join("src/large.rs"), &content).expect("write file");

    let db = GraphDb::open_in_memory().expect("open memory db");
    db.insert_repo("api", repo_dir.to_str().unwrap(), "main", None)
        .expect("insert repo");
    db.upsert_file("api", "src/large.rs", "h_large", 100, content.len() as i64)
        .expect("upsert file");

    // Request range 20-30
    let res = crate::action::graph::explore::explore_files(
        &db,
        hall_root,
        "src/large.rs:20-30",
        Some("api"),
    )
    .expect("explore_files with range");

    assert_eq!(res.sources.len(), 1);
    let source = &res.sources[0];
    assert_eq!(source.file_path, "src/large.rs");
    assert_eq!(source.excerpts.len(), 1);
    let excerpt = &source.excerpts[0];
    assert_eq!(excerpt.start_line, 20);
    assert_eq!(excerpt.end_line, 30);
    assert!(excerpt.code.contains("20\tfn line_20()"));
    assert!(excerpt.code.contains("30\tfn line_30()"));
    assert!(!excerpt.code.contains("19\tfn line_19()"));
    assert!(!excerpt.code.contains("31\tfn line_31()"));
}

#[test]
fn test_explore_ranged_path_invalid_range_errors() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let hall_root = temp.path();
    let repo_dir = hall_root.join("api");
    std::fs::create_dir_all(repo_dir.join("src")).expect("create dirs");
    let content = "fn main() {}\n";
    std::fs::write(repo_dir.join("src/main.rs"), content).expect("write file");

    let db = GraphDb::open_in_memory().expect("open memory db");
    db.insert_repo("api", repo_dir.to_str().unwrap(), "main", None)
        .expect("insert repo");
    db.upsert_file("api", "src/main.rs", "h_main", 1, content.len() as i64)
        .expect("upsert file");

    // Start == 0
    let err_zero = crate::action::graph::explore::explore_files(
        &db,
        hall_root,
        "src/main.rs:0-10",
        Some("api"),
    );
    assert!(err_zero.is_err(), "start=0 must error");

    // Start > End
    let err_order = crate::action::graph::explore::explore_files(
        &db,
        hall_root,
        "src/main.rs:20-10",
        Some("api"),
    );
    assert!(err_order.is_err(), "start > end must error");
}
#[test]
fn test_explore_ranged_path_hall_relative_matches() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let hall_root = temp.path();
    let repo_dir = hall_root.join(".ivar/repos/computron/main");
    std::fs::create_dir_all(repo_dir.join("src/core")).expect("create dirs");

    let lines: Vec<String> = (1..=300).map(|i| format!("// line {i}\n")).collect();
    let content = lines.concat();
    std::fs::write(repo_dir.join("src/core/prompt.ts"), &content).expect("write file");

    let db = GraphDb::open_in_memory().expect("open memory db");
    db.insert_repo("computron", repo_dir.to_str().unwrap(), "main", None)
        .expect("insert repo");
    db.upsert_file(
        "computron",
        "src/core/prompt.ts",
        "h_prompt",
        100,
        content.len() as i64,
    )
    .expect("upsert file");

    // Query using hall-relative path
    let res = crate::action::graph::explore::explore_files(
        &db,
        hall_root,
        ".ivar/repos/computron/main/src/core/prompt.ts:82-237",
        None,
    )
    .expect("explore_files with hall-relative ranged path");

    assert_eq!(res.sources.len(), 1);
    let source = &res.sources[0];
    assert_eq!(source.file_path, "src/core/prompt.ts");
    assert_eq!(source.excerpts.len(), 1);
    let excerpt = &source.excerpts[0];
    assert_eq!(excerpt.start_line, 82);
    assert_eq!(excerpt.end_line, 237);
    assert!(excerpt.code.contains("82\t// line 82"));
    assert!(excerpt.code.contains("237\t// line 237"));
    assert!(!excerpt.code.contains("81\t// line 81"));
    assert!(!excerpt.code.contains("238\t// line 238"));
}

#[test]
fn test_explore_ranged_path_boundary_matching() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let hall_root = temp.path();
    let repo_dir = hall_root.join("api");
    std::fs::create_dir_all(repo_dir.join("src")).expect("create dirs");

    let lines_data: Vec<String> = (1..=50).map(|i| format!("// data line {i}\n")).collect();
    let content_data = lines_data.concat();
    std::fs::write(repo_dir.join("src/data.ts"), &content_data).expect("write file");

    let lines_a: Vec<String> = (1..=50).map(|i| format!("// a line {i}\n")).collect();
    let content_a = lines_a.concat();
    std::fs::write(repo_dir.join("src/a.ts"), &content_a).expect("write file");

    let db = GraphDb::open_in_memory().expect("open memory db");
    db.insert_repo("api", repo_dir.to_str().unwrap(), "main", None)
        .expect("insert repo");
    db.upsert_file(
        "api",
        "src/data.ts",
        "h_data",
        50,
        content_data.len() as i64,
    )
    .expect("upsert data.ts");
    db.upsert_file("api", "src/a.ts", "h_a", 50, content_a.len() as i64)
        .expect("upsert a.ts");

    // Ranged query for "a.ts:10-20 src/data.ts"
    let res = crate::action::graph::explore::explore_files(
        &db,
        hall_root,
        "a.ts:10-20 src/data.ts",
        Some("api"),
    )
    .expect("explore_files with ranged a.ts and whole data.ts");

    let source_a = res
        .sources
        .iter()
        .find(|s| s.file_path == "src/a.ts")
        .expect("find src/a.ts");
    assert_eq!(source_a.excerpts.len(), 1);
    assert_eq!(source_a.excerpts[0].start_line, 10);
    assert_eq!(source_a.excerpts[0].end_line, 20);

    let source_data = res
        .sources
        .iter()
        .find(|s| s.file_path == "src/data.ts")
        .expect("find src/data.ts");
    // data.ts was not given a line range, so it must not have matched "a.ts" range
    assert_eq!(source_data.excerpts[0].start_line, 1);
    assert_eq!(source_data.excerpts[0].end_line, 50);
}

#[test]
fn test_explore_ranged_path_slices_primary_symbols() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let hall_root = temp.path();
    let repo_dir = hall_root.join("api");
    std::fs::create_dir_all(repo_dir.join("src")).expect("create dirs");

    let lines: Vec<String> = (1..=200)
        .map(|i| {
            if i == 5 {
                "pub fn big_func() {\n".to_owned()
            } else if i == 150 {
                "}\n".to_owned()
            } else {
                format!("    // step {i}\n")
            }
        })
        .collect();
    let content = lines.concat();
    std::fs::write(repo_dir.join("src/large.rs"), &content).expect("write file");

    let db = GraphDb::open_in_memory().expect("open memory db");
    db.insert_repo("api", repo_dir.to_str().unwrap(), "main", None)
        .expect("insert repo");
    let file_id = db
        .upsert_file("api", "src/large.rs", "h_large", 200, content.len() as i64)
        .expect("upsert file");
    db.insert_symbols(&[fn_symbol(file_id, "big_func", 5, 150)])
        .expect("insert symbol");

    // Explore query for "src/large.rs:20-30"
    let res =
        crate::action::graph::explore::explore(&db, hall_root, "src/large.rs:20-30", Some("api"))
            .expect("explore with range");

    assert_eq!(res.primary_symbols.len(), 1);
    let sym = &res.primary_symbols[0];
    assert_eq!(sym.symbol.name, "big_func");
    assert_eq!(sym.start_line, 20);
    assert_eq!(sym.end_line, 30);
    assert!(sym.code.contains("20\t    // step 20"));
    assert!(sym.code.contains("30\t    // step 30"));
    assert!(!sym.code.contains("19\t"));
    assert!(!sym.code.contains("31\t"));
}

#[test]
fn a_hall_file_is_returned_by_its_hall_relative_path_and_by_content_with_repo_hall() {
    let temp = tempdir().expect("create temp dir");
    let root = temp.path();
    fs::create_dir_all(root.join("brand")).unwrap();
    fs::write(
        root.join("brand/DESIGN.md"),
        "# design\n\nthe chariot mark\n",
    )
    .unwrap();
    let db = GraphDb::open_in_memory().unwrap();
    db.ensure_views_base_mode().unwrap();
    crate::action::graph::index::index_hall_path(
        &db,
        root,
        "brand",
        false,
        &crate::infra::progress::Silent,
    )
    .unwrap();

    let by_path = explore(&db, root, "brand/DESIGN.md", None).unwrap();
    let hit = by_path
        .file_matches
        .iter()
        .find(|f| f.file_path == "brand/DESIGN.md")
        .expect("file match by path");
    assert_eq!(hit.repo, "hall");

    let by_content = explore(&db, root, "chariot mark", Some("hall")).unwrap();
    assert!(
        by_content
            .file_matches
            .iter()
            .any(|f| f.file_path == "brand/DESIGN.md"),
        "{by_content:?}"
    );
}

fn file_matches_of(result: &ExploreResult) -> Vec<(String, String)> {
    result
        .file_matches
        .iter()
        .map(|f| (f.repo.clone(), f.file_path.clone()))
        .collect()
}

#[test]
fn a_hall_path_never_resolves_to_a_shorter_path_in_another_repo() {
    let fixture = explore_fixture();
    fixture.index_text("valhalla", "DESIGN.md", "# valhalla design\n");
    fixture.index_text("cheat-sheets", "index.html", "<html>cheat</html>\n");

    for query in ["brand/DESIGN.md", "brand/moodboard/index.html"] {
        let result = explore(&fixture.db, fixture.root(), query, None).unwrap();
        assert!(result.file_matches.is_empty(), "{query}: {result:?}");
        assert!(result.sources.is_empty(), "{query}: {result:?}");
        let summary = result.impact_summary.unwrap();
        assert!(
            summary.contains("No indexed file or directory"),
            "{summary}"
        );
    }
}

#[test]
fn an_indexed_hall_path_wins_over_a_repo_file_with_the_same_basename() {
    let fixture = explore_fixture();
    fixture.index_text("valhalla", "DESIGN.md", "# valhalla design\n");
    fixture.index_text("hall", "brand/DESIGN.md", "# brand design\n");

    let result = explore(&fixture.db, fixture.root(), "brand/DESIGN.md", None).unwrap();

    assert_eq!(
        file_matches_of(&result),
        vec![("hall".to_owned(), "brand/DESIGN.md".to_owned())]
    );
}

#[test]
fn an_exact_path_in_several_repos_returns_those_files_and_no_suffix_neighbours() {
    let fixture = explore_fixture();
    fixture.index_text("api", ".github/workflows/ci.yml", "name: api\n");
    fixture.index_text("web", ".github/workflows/ci.yml", "name: web\n");
    fixture.index_text("web", "tools/.github/workflows/ci.yml", "name: nested\n");

    let result = explore(
        &fixture.db,
        fixture.root(),
        ".github/workflows/ci.yml",
        None,
    )
    .unwrap();

    let mut found = file_matches_of(&result);
    found.sort();
    assert_eq!(
        found,
        vec![
            ("api".to_owned(), ".github/workflows/ci.yml".to_owned()),
            ("web".to_owned(), ".github/workflows/ci.yml".to_owned()),
        ]
    );
}

#[test]
fn a_directory_glob_names_the_files_under_the_directory() {
    let fixture = explore_fixture();
    fixture.index_text("hall", "brand/wayfinder/gtm/plan.md", "# plan\n");
    fixture.index_text("hall", "brand/wayfinder/gtm/notes/a.md", "# a\n");
    fixture.index_text("hall", "brand/other.md", "# other\n");

    for query in [
        "brand/wayfinder/gtm/**",
        "brand/wayfinder/gtm/*",
        "brand/wayfinder/gtm/",
    ] {
        let result = explore(&fixture.db, fixture.root(), query, None).unwrap();
        let mut found: Vec<String> = result
            .file_matches
            .into_iter()
            .map(|f| f.file_path)
            .collect();
        found.sort();
        assert_eq!(
            found,
            vec![
                "brand/wayfinder/gtm/notes/a.md",
                "brand/wayfinder/gtm/plan.md"
            ],
            "{query}"
        );
    }
}

#[test]
fn an_unindexed_path_is_not_answered_with_unrelated_symbols() {
    let (db, temp) = index_single_file(
        "pub fn mcp_server() {}\n",
        "src/mcp.rs",
        &[("mcp_server", 1, 1)],
    );

    let result = explore(
        &db,
        temp.path(),
        "brand/wayfinder/mcp-go-to-market/**",
        None,
    )
    .unwrap();

    assert!(result.primary_symbols.is_empty(), "{result:?}");
    assert!(result.file_matches.is_empty(), "{result:?}");
    assert!(result.sources.is_empty(), "{result:?}");
}

#[test]
fn manifests_lockfiles_infra_and_extensionless_files_are_resolved_by_name() {
    let fixture = explore_fixture();
    fixture.index_text(
        "infra",
        "terraform/main.tf",
        "resource \"aws_security_group\" \"sg\" {}\n",
    );
    fixture.index_text("infra", "Makefile", "build:\n\tcargo build\n");
    fixture.index_text("web", "pnpm-lock.yaml", "lockfileVersion: 9\n");
    fixture.index_text("web", "package.json", "{\"name\": \"web\"}\n");

    for (query, path) in [
        ("main.tf", "terraform/main.tf"),
        ("Makefile", "Makefile"),
        ("pnpm-lock.yaml", "pnpm-lock.yaml"),
        ("package.json", "package.json"),
    ] {
        let result = explore(&fixture.db, fixture.root(), query, None).unwrap();
        let first = result
            .file_matches
            .first()
            .unwrap_or_else(|| panic!("{query}: {result:?}"));
        assert_eq!(first.file_path, path, "{query}");
        assert!(!result.sources.is_empty(), "{query}");
    }
}

#[test]
fn is_path_like_recognises_files_by_extension_and_well_known_name() {
    use crate::action::graph::query::find::is_path_like;

    for token in [
        "main.tf",
        "Makefile",
        "Dockerfile",
        "CODEOWNERS",
        "a.hcl",
        "vite.config.ts",
    ] {
        assert!(is_path_like(token), "{token}");
    }
    for token in [
        "makefile",
        "GraphDb",
        "1.5",
        "v2.0",
        ".",
        "trailing.",
        "plain",
    ] {
        assert!(!is_path_like(token) || token == ".", "{token}");
    }
}

#[test]
fn a_workspace_prefix_naming_another_repo_does_not_resolve_to_this_repos_file() {
    let fixture = explore_fixture();
    fixture.index_text("api", "src/auth.ts", "export const auth = 1;\n");

    let named = explore(
        &fixture.db,
        fixture.root(),
        "services/api/src/auth.ts",
        None,
    )
    .unwrap();
    assert_eq!(
        file_matches_of(&named),
        vec![("api".to_owned(), "src/auth.ts".to_owned())]
    );

    let other = explore(
        &fixture.db,
        fixture.root(),
        "services/web/src/auth.ts",
        None,
    )
    .unwrap();
    assert!(other.file_matches.is_empty(), "{other:?}");
    assert!(other.sources.is_empty(), "{other:?}");
}

#[test]
fn a_miss_in_an_unindexed_hall_directory_points_at_graph_paths() {
    let fixture = explore_fixture();
    fixture.index_text("valhalla", "DESIGN.md", "# valhalla design\n");
    std::fs::create_dir_all(fixture.root().join("brand")).unwrap();
    std::fs::write(fixture.root().join("brand/DESIGN.md"), "# brand\n").unwrap();

    let result = explore(&fixture.db, fixture.root(), "brand/DESIGN.md", None).unwrap();

    assert!(result.file_matches.is_empty(), "{result:?}");
    assert!(result.sources.is_empty(), "{result:?}");
    let summary = result.impact_summary.unwrap();
    assert!(summary.contains("graph.paths"), "{summary}");
    assert!(summary.contains("--path brand"), "{summary}");
    assert!(!summary.contains("valhalla"), "{summary}");
}

#[test]
fn a_miss_in_an_indexed_hall_directory_does_not_ask_to_list_it_again() {
    let fixture = explore_fixture();
    fixture.index_text("hall", "brand/GUIDELINE.md", "# guideline\n");
    std::fs::create_dir_all(fixture.root().join("brand")).unwrap();

    let result = explore(&fixture.db, fixture.root(), "brand/DESIGN.md", None).unwrap();

    let summary = result.impact_summary.unwrap();
    assert!(summary.contains("`brand` is indexed"), "{summary}");
    assert!(!summary.contains("does not index"), "{summary}");
}

#[test]
fn a_hall_file_the_graph_skipped_is_reported_as_present_on_disk() {
    let fixture = explore_fixture();
    fixture.index_text("hall", "brand/GUIDELINE.md", "# guideline\n");
    std::fs::create_dir_all(fixture.root().join("brand")).unwrap();
    std::fs::write(fixture.root().join("brand/logo.png"), [0u8, 159, 146, 150]).unwrap();

    let result = explore(&fixture.db, fixture.root(), "brand/logo.png", None).unwrap();

    let summary = result.impact_summary.unwrap();
    assert!(summary.contains("exists in the hall"), "{summary}");
}

#[test]
fn a_basename_without_a_directory_still_matches_across_repos_and_names_each_repo() {
    let fixture = explore_fixture();
    fixture.index_text("api", "docs/DESIGN.md", "# api design\n");
    fixture.index_text("web", "notes/DESIGN.md", "# web design\n");

    let result = explore(&fixture.db, fixture.root(), "DESIGN.md", None).unwrap();

    let mut found = file_matches_of(&result);
    found.sort();
    assert_eq!(
        found,
        vec![
            ("api".to_owned(), "docs/DESIGN.md".to_owned()),
            ("web".to_owned(), "notes/DESIGN.md".to_owned()),
        ]
    );
}

#[test]
fn exact_paths_of_manifests_workflows_and_infra_resolve_inside_their_repo() {
    let fixture = explore_fixture();
    for repo in ["web", "infra"] {
        fixture.index_text(repo, "package.json", "{\"name\": \"x\"}\n");
        fixture.index_text(repo, ".github/workflows/ci.yml", "name: ci\njobs: {}\n");
    }
    fixture.index_text("web", "apps/console/vite.config.ts", "export default {};\n");
    fixture.index_text(
        "infra",
        "terraform/main.tf",
        "resource \"aws_iam_role\" \"r\" {}\n",
    );
    fixture.index_text("infra", "terraform/.terraform.lock.hcl", "provider {}\n");

    for (repo, path) in [
        ("web", "package.json"),
        ("infra", ".github/workflows/ci.yml"),
        ("web", "apps/console/vite.config.ts"),
        ("infra", "terraform/main.tf"),
        ("infra", "terraform/.terraform.lock.hcl"),
    ] {
        let result = explore(&fixture.db, fixture.root(), path, Some(repo)).unwrap();
        assert_eq!(
            file_matches_of(&result),
            vec![(repo.to_owned(), path.to_owned())],
            "{repo}: {path}"
        );
        assert!(!result.sources.is_empty(), "{repo}: {path}");
    }
}
