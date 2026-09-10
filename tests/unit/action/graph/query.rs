#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::graph::{EdgeKind, Provenance, Span, SymbolKind};

fn setup_test_db() -> (GraphDb, i64, i64, i64) {
    let db = GraphDb::open_in_memory().expect("open in-memory db");
    db.insert_repo("ivar", "/path/to/ivar", "main", None)
        .expect("insert repo");

    // Insert file 1: src/lib.rs
    let file1_id = db
        .upsert_file("ivar", "src/lib.rs", "hash1", 1000, 500)
        .expect("upsert file 1");

    // Insert symbols: helper and caller_fn in src/lib.rs
    let symbols = vec![
        Symbol {
            id: None,
            file_id: Some(file1_id),
            repo: "ivar".to_owned(),
            name: "helper".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn helper()".to_owned()),
            docstring: Some("A helper function".to_owned()),
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: Some(2),
        },
        Symbol {
            id: None,
            file_id: Some(file1_id),
            repo: "ivar".to_owned(),
            name: "caller_fn".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn caller_fn()".to_owned()),
            docstring: Some("A caller function".to_owned()),
            span: Span::new(7, 1, 15, 1),
            is_exported: true,
            complexity: Some(5),
        },
    ];
    let sym_ids = db.insert_symbols(&symbols).expect("insert symbols");
    let helper_id = sym_ids[0];
    let caller_fn_id = sym_ids[1];

    // Insert file 2: src/other.rs
    let file2_id = db
        .upsert_file("ivar", "src/other.rs", "hash2", 2000, 300)
        .expect("upsert file 2");

    // Insert symbol: top_fn in src/other.rs
    let top_syms = vec![Symbol {
        id: None,
        file_id: Some(file2_id),
        repo: "ivar".to_owned(),
        name: "top_fn".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("fn top_fn()".to_owned()),
        docstring: None,
        span: Span::new(1, 1, 10, 1),
        is_exported: false,
        complexity: Some(8),
    }];
    let top_ids = db.insert_symbols(&top_syms).expect("insert top_fn");
    let top_fn_id = top_ids[0];

    // Insert edges:
    // 1. Import edge in src/lib.rs
    // 2. caller_fn -> helper
    // 3. top_fn -> caller_fn
    let edges = vec![
        Edge {
            id: None,
            repo: "ivar".to_owned(),
            file_id: Some(file1_id),
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("std::io".to_owned()),
            kind: EdgeKind::Imports,
            provenance: Provenance::Extracted,
            line: 1,
            col: 1,
            confidence: 1.0,
        },
        Edge {
            id: None,
            repo: "ivar".to_owned(),
            file_id: Some(file1_id),
            from_symbol_id: Some(caller_fn_id),
            to_symbol_id: Some(helper_id),
            to_name: Some("helper".to_owned()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 10,
            col: 5,
            confidence: 0.95,
        },
        Edge {
            id: None,
            repo: "ivar".to_owned(),
            file_id: Some(file2_id),
            from_symbol_id: Some(top_fn_id),
            to_symbol_id: Some(caller_fn_id),
            to_name: Some("caller_fn".to_owned()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 5,
            col: 5,
            confidence: 0.9,
        },
    ];
    db.insert_edges(&edges).expect("insert edges");

    (db, helper_id, caller_fn_id, top_fn_id)
}

#[test]
fn test_find_symbols_exact_and_prefix() {
    let (db, helper_id, caller_fn_id, _) = setup_test_db();

    // Exact match
    let exact = find_symbols(&db, "helper", Some("ivar"), 10).expect("find exact");
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].symbol.id, Some(helper_id));
    assert_eq!(exact[0].symbol.name, "helper");
    assert_eq!(exact[0].symbol.complexity, Some(2));
    assert_eq!(exact[0].file_path, "src/lib.rs");

    // Prefix match
    let prefix = find_symbols(&db, "call", None, 10).expect("find prefix");
    assert_eq!(prefix.len(), 1);
    assert_eq!(prefix[0].symbol.id, Some(caller_fn_id));
    assert_eq!(prefix[0].symbol.name, "caller_fn");

    assert_eq!(prefix[0].symbol.complexity, Some(5));
    // Non-existent symbol
    let empty = find_symbols(&db, "non_existent", None, 10).expect("find non existent");
    assert!(empty.is_empty());
}

#[test]
fn test_get_callers() {
    let (db, _helper_id, caller_fn_id, _) = setup_test_db();

    let callers = get_callers(&db, "helper", Some("ivar"), false, 0.5).expect("get callers");
    assert_eq!(callers.len(), 1);
    assert_eq!(callers[0].caller.id, Some(caller_fn_id));
    assert_eq!(callers[0].caller.name, "caller_fn");
    assert_eq!(callers[0].caller_file_path, "src/lib.rs");
    assert_eq!(callers[0].caller.complexity, Some(5));
    assert_eq!(callers[0].edge_kind, EdgeKind::Calls);
    assert_eq!(callers[0].line, 10);
    assert_eq!(callers[0].col, 5);

    // Filter by high confidence
    let filtered = get_callers(&db, "helper", Some("ivar"), false, 0.99).expect("filtered callers");
    assert!(filtered.is_empty());
}

#[test]
fn test_get_callees() {
    let (db, helper_id, caller_fn_id, _) = setup_test_db();

    let callees = get_callees(&db, caller_fn_id).expect("get callees");
    assert_eq!(callees.len(), 1);
    assert_eq!(callees[0].callee_name, "helper");
    assert_eq!(
        callees[0].callee_symbol.as_ref().and_then(|s| s.id),
        Some(helper_id)
    );
    assert_eq!(
        callees[0]
            .callee_symbol
            .as_ref()
            .and_then(|symbol| symbol.complexity),
        Some(2)
    );
    assert_eq!(callees[0].callee_file_path.as_deref(), Some("src/lib.rs"));
    assert_eq!(callees[0].edge_kind, EdgeKind::Calls);

    // helper has no outgoing calls
    let helper_callees = get_callees(&db, helper_id).expect("helper callees");
    assert!(helper_callees.is_empty());
}

#[test]
fn test_get_file_outline() {
    let (db, _, _, _) = setup_test_db();

    let outline = get_file_outline(&db, "ivar", "src/lib.rs").expect("file outline");
    assert_eq!(outline.file_path, "src/lib.rs");
    assert_eq!(outline.repo, "ivar");
    assert_eq!(outline.symbols.len(), 2);
    assert_eq!(outline.symbols[0].name, "helper");
    assert_eq!(outline.symbols[1].name, "caller_fn");
    assert_eq!(outline.imports.len(), 1);
    assert_eq!(outline.symbols[0].complexity, Some(2));
    assert_eq!(outline.symbols[1].complexity, Some(5));
    assert_eq!(outline.imports[0].to_name.as_deref(), Some("std::io"));

    // File not found error
    let err = get_file_outline(&db, "ivar", "src/missing.rs").unwrap_err();
    match err {
        QueryError::FileNotFound { repo, path } => {
            assert_eq!(repo, "ivar");
            assert_eq!(path, "src/missing.rs");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn test_get_graph_stats() {
    let (db, _, _, _) = setup_test_db();

    let stats = get_graph_stats(&db).expect("get stats");
    assert_eq!(stats.repo_count, 1);
    assert_eq!(stats.file_count, 2);
    assert_eq!(stats.symbol_count, 3);
    assert_eq!(stats.edge_count, 3);
}

#[test]
fn test_get_impact_and_cycle_protection() {
    let (db, helper_id, caller_fn_id, top_fn_id) = setup_test_db();

    // Impact of helper:
    // caller_fn calls helper (depth 1)
    // top_fn calls caller_fn (depth 2)
    let impact = get_impact(&db, helper_id, 5).expect("get impact");
    assert_eq!(impact.root_symbol.id, Some(helper_id));
    assert_eq!(impact.root_symbol.complexity, Some(2));
    assert_eq!(impact.total_affected, 2);
    assert_eq!(impact.affected_symbols.len(), 2);
    assert_eq!(impact.affected_files, vec!["src/lib.rs", "src/other.rs"]);

    assert_eq!(impact.affected_symbols[0].symbol.id, Some(caller_fn_id));
    assert_eq!(impact.affected_symbols[0].symbol.complexity, Some(5));
    assert_eq!(impact.affected_symbols[0].depth, 1);
    assert_eq!(
        impact.affected_symbols[0].path_via,
        vec!["helper", "caller_fn"]
    );

    assert_eq!(impact.affected_symbols[1].symbol.id, Some(top_fn_id));
    assert_eq!(impact.affected_symbols[1].symbol.complexity, Some(8));
    assert_eq!(impact.affected_symbols[1].depth, 2);
    assert_eq!(
        impact.affected_symbols[1].path_via,
        vec!["helper", "caller_fn", "top_fn"]
    );

    // Impact with max_depth = 1
    let shallow = get_impact(&db, helper_id, 1).expect("shallow impact");
    assert_eq!(shallow.total_affected, 1);
    assert_eq!(shallow.affected_symbols[0].symbol.id, Some(caller_fn_id));

    // Add a cycle: helper calls top_fn
    let file1_id = impact.root_symbol.file_id.unwrap();
    let cycle_edge = Edge {
        id: None,
        repo: "ivar".to_owned(),
        file_id: Some(file1_id),
        from_symbol_id: Some(helper_id),
        to_symbol_id: Some(top_fn_id),
        to_name: Some("top_fn".to_owned()),
        kind: EdgeKind::Calls,
        provenance: Provenance::Extracted,
        line: 3,
        col: 5,
        confidence: 1.0,
    };
    db.insert_edges(&[cycle_edge]).expect("insert cycle edge");

    // Traverse with cycle should terminate cleanly without infinite recursion
    let cycle_impact = get_impact(&db, helper_id, 10).expect("cycle impact");
    assert_eq!(cycle_impact.total_affected, 2);
}

fn function_symbol(file_id: i64, repo: &str, name: &str, start_line: usize) -> Symbol {
    Symbol {
        id: None,
        file_id: Some(file_id),
        repo: repo.to_owned(),
        name: name.to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some(format!("fn {name}()")),
        docstring: None,
        span: Span::new(start_line, 1, start_line + 5, 1),
        is_exported: true,
        complexity: None,
    }
}

#[test]
fn test_explore_find_candidates_path_pinning_and_per_file_limits() {
    let db = GraphDb::open_in_memory().expect("open in-memory db");
    db.insert_repo("myrepo", "/workspace/myrepo", "main", None)
        .expect("insert repo");

    // File 1: src/auth/handler.rs
    let file1_id = db
        .upsert_file("myrepo", "src/auth/handler.rs", "h1", 100, 1000)
        .expect("upsert file 1");

    // Insert 10 symbols into file 1
    let mut syms_f1 = Vec::new();
    for i in 1..=10 {
        syms_f1.push(function_symbol(
            file1_id,
            "myrepo",
            &format!("login_step_{i}"),
            i * 10,
        ));
    }
    db.insert_symbols(&syms_f1).expect("insert f1 symbols");

    // File 2: src/auth/middleware.rs
    let file2_id = db
        .upsert_file("myrepo", "src/auth/middleware.rs", "h2", 100, 1000)
        .expect("upsert file 2");

    let mut syms_f2 = Vec::new();
    for i in 1..=8 {
        syms_f2.push(function_symbol(
            file2_id,
            "myrepo",
            &format!("auth_mw_step_{i}"),
            i * 10,
        ));
    }
    db.insert_symbols(&syms_f2).expect("insert f2 symbols");

    // 1. Exact file pinning (single file matched) -> can return up to MAX_EXPLORE_CANDIDATES (all 10 symbols)
    let c_exact =
        find::explore_find_candidates(&db, "src/auth/handler.rs", None).expect("explore exact");
    assert_eq!(c_exact.len(), 10);
    assert!(c_exact.iter().all(|s| s.file_path == "src/auth/handler.rs"));
    // Verify source line order is preserved
    for i in 0..9 {
        assert!(c_exact[i].symbol.span.start_line < c_exact[i + 1].symbol.span.start_line);
    }

    // 2. Unambiguous basename pinning -> "handler.rs" resolves to src/auth/handler.rs
    let c_base =
        find::explore_find_candidates(&db, "handler.rs login", None).expect("explore basename");
    assert_eq!(c_base.len(), 10);
    assert!(c_base.iter().all(|s| s.file_path == "src/auth/handler.rs"));

    // 3. Workspace-relative path pinning -> "myrepo/src/auth/middleware.rs"
    let c_ws = find::explore_find_candidates(&db, "myrepo/src/auth/middleware.rs", None)
        .expect("explore ws path");
    assert_eq!(c_ws.len(), 8);
    assert!(c_ws.iter().all(|s| s.file_path == "src/auth/middleware.rs"));

    // 4. Directory subtree query -> "src/auth" matches both files.
    // Because multiple files match, each file is capped at MAX_SYMBOLS_PER_FILE (6).
    let c_dir = find::explore_find_candidates(&db, "src/auth", None).expect("explore dir");
    assert_eq!(c_dir.len(), 12); // 6 from handler.rs + 6 from middleware.rs
    let f1_count = c_dir
        .iter()
        .filter(|s| s.file_path == "src/auth/handler.rs")
        .count();
    let f2_count = c_dir
        .iter()
        .filter(|s| s.file_path == "src/auth/middleware.rs")
        .count();
    assert_eq!(f1_count, 6);
    assert_eq!(f2_count, 6);
}

#[test]
fn test_explore_find_candidates_keeps_best_match_past_file_cap() {
    let db = GraphDb::open_in_memory().expect("open in-memory db");
    db.insert_repo("bigrepo", "/workspace/bigrepo", "main", None)
        .expect("insert repo");

    let file_id = db
        .upsert_file("bigrepo", "src/big.rs", "h", 10, 100)
        .expect("upsert file");

    let mut symbols: Vec<Symbol> = (1..=find::MAX_EXPLORE_CANDIDATES + 6)
        .map(|i| function_symbol(file_id, "bigrepo", &format!("step_{i}"), i * 10))
        .collect();
    symbols.push(function_symbol(file_id, "bigrepo", "authenticate", 10_000));
    db.insert_symbols(&symbols).expect("insert symbols");

    let candidates = find::explore_find_candidates(&db, "src/big.rs authenticate", None)
        .expect("search candidates");

    assert_eq!(candidates.len(), find::MAX_EXPLORE_CANDIDATES);
    assert!(candidates.iter().any(|c| c.symbol.name == "authenticate"));
}

#[test]
fn test_explore_find_candidates_weighted_scoring_and_terms() {
    let db = GraphDb::open_in_memory().expect("open in-memory db");
    db.insert_repo("testrepo", "/workspace/testrepo", "main", None)
        .expect("insert repo");

    let file_id = db
        .upsert_file("testrepo", "src/auth.rs", "h", 10, 100)
        .expect("upsert file");

    let sym_exact = Symbol {
        docstring: Some("Perform user authentication".to_owned()),
        ..function_symbol(file_id, "testrepo", "authenticate", 10)
    };
    let sym_prefix = function_symbol(file_id, "testrepo", "authenticate_with_token", 20);
    let sym_doc_only = Symbol {
        docstring: Some("Checks if user authentication is valid".to_owned()),
        ..function_symbol(file_id, "testrepo", "validate_session", 30)
    };

    db.insert_symbols(&[sym_exact, sym_prefix, sym_doc_only])
        .expect("insert symbols");

    // Search "authenticate" with path "src/auth.rs"
    let candidates = find::explore_find_candidates(&db, "src/auth.rs authenticate", None)
        .expect("search candidates");

    assert_eq!(candidates.len(), 3);
    // The exact match and prefix match should be present
    let names: Vec<_> = candidates.iter().map(|c| c.symbol.name.as_str()).collect();
    assert!(names.contains(&"authenticate"));
    assert!(names.contains(&"authenticate_with_token"));
    assert!(names.contains(&"validate_session"));
}

#[test]
fn test_explore_find_candidates_admits_every_pinned_file() {
    let db = GraphDb::open_in_memory().expect("open in-memory db");
    db.insert_repo("api", "/workspace/api", "main", None)
        .expect("insert repo");

    let mut paths = Vec::new();
    for f in 0..7 {
        let path = format!("src/routes/r{f}.ts");
        let file_id = db
            .upsert_file("api", &path, "h", 10, 100)
            .expect("upsert file");
        let symbols: Vec<Symbol> = (1..=8)
            .map(|i| function_symbol(file_id, "api", &format!("r{f}_handler{i}"), i * 10))
            .collect();
        db.insert_symbols(&symbols).expect("insert symbols");
        paths.push(path);
    }

    let candidates =
        find::explore_find_candidates(&db, &paths.join(" "), None).expect("search candidates");

    let files: std::collections::BTreeSet<&str> =
        candidates.iter().map(|c| c.file_path.as_str()).collect();
    assert_eq!(
        files.len(),
        7,
        "every file the query names must be represented"
    );
    assert!(candidates.len() <= find::MAX_EXPLORE_CANDIDATES);
}

#[test]
fn test_explore_find_candidates_prefers_a_matching_directory_over_prefix_decoys() {
    let db = GraphDb::open_in_memory().expect("open in-memory db");
    db.insert_repo("api", "/workspace/api", "main", None)
        .expect("insert repo");

    let files: [(&str, &[&str]); 3] = [
        (
            "src/auth/sessions.ts",
            &["createSession", "getSession", "destroySession"],
        ),
        ("src/auth/credentials.ts", &["verifyCredentials"]),
        ("src/lib/noise.ts", &["authorizePayment", "formatBytes"]),
    ];
    for (path, names) in files {
        let file_id = db
            .upsert_file("api", path, "h", 10, 100)
            .expect("upsert file");
        let symbols: Vec<Symbol> = names
            .iter()
            .enumerate()
            .map(|(i, name)| function_symbol(file_id, "api", name, (i + 1) * 10))
            .collect();
        db.insert_symbols(&symbols).expect("insert symbols");
    }

    let candidates = find::explore_find_candidates(&db, "auth", None).expect("search candidates");

    let found: std::collections::BTreeSet<&str> =
        candidates.iter().map(|c| c.file_path.as_str()).collect();
    assert!(found.contains("src/auth/sessions.ts"), "got {found:?}");
    assert!(found.contains("src/auth/credentials.ts"), "got {found:?}");
    assert!(
        !found.contains("src/lib/noise.ts"),
        "a name that only starts with the term does not match it: {found:?}"
    );
}

#[test]
fn test_explore_find_candidates_matches_a_singular_term_to_a_plural_path() {
    let db = GraphDb::open_in_memory().expect("open in-memory db");
    db.insert_repo("api", "/workspace/api", "main", None)
        .expect("insert repo");
    for (path, name) in [
        ("src/auth/sessions.ts", "createSession"),
        ("src/lib/noise.ts", "formatBytes"),
    ] {
        let file_id = db
            .upsert_file("api", path, "h", 10, 100)
            .expect("upsert file");
        db.insert_symbols(&[function_symbol(file_id, "api", name, 10)])
            .expect("insert symbol");
    }

    let candidates =
        find::explore_find_candidates(&db, "session", None).expect("search candidates");

    let found: Vec<&str> = candidates.iter().map(|c| c.file_path.as_str()).collect();
    assert_eq!(found, vec!["src/auth/sessions.ts"]);
}

#[test]
fn test_explore_find_keeps_source_to_the_best_files_and_names_the_rest() {
    let db = GraphDb::open_in_memory().expect("open in-memory db");
    db.insert_repo("api", "/workspace/api", "main", None)
        .expect("insert repo");
    for i in 0..6 {
        let file_id = db
            .upsert_file("api", &format!("src/routes/r{i}.ts"), "h", 10, 100)
            .expect("upsert file");
        db.insert_symbols(&[function_symbol(
            file_id,
            "api",
            &format!("route{i}Handler"),
            10,
        )])
        .expect("insert symbol");
    }

    let found = find::explore_find(&db, "src/routes", None, 4).expect("explore find");

    let shown: std::collections::BTreeSet<&str> =
        found.symbols.iter().map(|c| c.file_path.as_str()).collect();
    assert_eq!(shown.len(), 4);
    assert_eq!(found.not_shown.len(), 2);
    assert!(
        found
            .not_shown
            .iter()
            .all(|file| { !shown.contains(file.file_path.as_str()) && !file.symbols.is_empty() })
    );
}

#[test]
fn test_explore_find_candidates_keeps_a_workspace_directory_inside_the_repo_it_names() {
    let db = GraphDb::open_in_memory().expect("open in-memory db");
    for repo in ["api", "web"] {
        db.insert_repo(repo, &format!("/workspace/{repo}"), "main", None)
            .expect("insert repo");
    }
    for (repo, path, name) in [
        ("api", "src/routes/auth.ts", "authRoutes"),
        ("web", "src/routes/registry.ts", "ROUTES"),
    ] {
        let file_id = db
            .upsert_file(repo, path, "h", 10, 100)
            .expect("upsert file");
        db.insert_symbols(&[function_symbol(file_id, repo, name, 10)])
            .expect("insert symbol");
    }

    let candidates = find::explore_find_candidates(&db, "services/api/src/routes", None)
        .expect("search candidates");

    let found: Vec<(&str, &str)> = candidates
        .iter()
        .map(|c| (c.symbol.repo.as_str(), c.file_path.as_str()))
        .collect();
    assert_eq!(found, vec![("api", "src/routes/auth.ts")]);
}

#[test]
fn test_explore_find_candidates_finds_a_word_inside_a_camel_case_name() {
    let db = GraphDb::open_in_memory().expect("open in-memory db");
    db.insert_repo("api", "/workspace/api", "main", None)
        .expect("insert repo");
    for (path, name) in [
        ("src/plugins/guard.ts", "enforceSession"),
        ("src/lib/noise.ts", "formatBytes"),
    ] {
        let file_id = db
            .upsert_file("api", path, "h", 10, 100)
            .expect("upsert file");
        db.insert_symbols(&[function_symbol(file_id, "api", name, 10)])
            .expect("insert symbol");
    }

    let candidates =
        find::explore_find_candidates(&db, "session", None).expect("search candidates");

    let found: Vec<&str> = candidates.iter().map(|c| c.symbol.name.as_str()).collect();
    assert_eq!(found, vec!["enforceSession"]);
}

#[test]
fn test_explore_find_candidates_ranks_a_test_file_below_the_code_it_tests() {
    let db = GraphDb::open_in_memory().expect("open in-memory db");
    db.insert_repo("api", "/workspace/api", "main", None)
        .expect("insert repo");
    let test_file = db
        .upsert_file("api", "tests/auth/session.test.ts", "h", 10, 100)
        .expect("upsert test file");
    let source_file = db
        .upsert_file("api", "src/auth/session.ts", "h", 10, 100)
        .expect("upsert source file");
    db.insert_symbols(&[
        function_symbol(test_file, "api", "createSession", 10),
        function_symbol(test_file, "api", "createSessionFixture", 20),
        function_symbol(source_file, "api", "createSession", 10),
    ])
    .expect("insert symbols");

    let candidates =
        find::explore_find_candidates(&db, "createSession", None).expect("search candidates");

    assert_eq!(
        candidates.first().map(|c| c.file_path.as_str()),
        Some("src/auth/session.ts")
    );
}

#[test]
fn test_get_references_lists_module_level_imports_of_a_symbol() {
    use crate::domain::graph::{Edge, EdgeKind, Provenance};

    let db = GraphDb::open_in_memory().expect("open in-memory db");
    db.insert_repo("web", "/workspace/web", "main", None)
        .expect("insert repo");
    let access = db
        .upsert_file("web", "src/auth/access.ts", "h", 10, 100)
        .expect("upsert access");
    let client = db
        .upsert_file("web", "src/api/client.ts", "h", 10, 100)
        .expect("upsert client");
    let session_id = db
        .insert_symbols(&[function_symbol(access, "web", "Session", 8)])
        .expect("insert symbol")
        .into_iter()
        .next()
        .expect("symbol id");
    let edge = |to_symbol_id, to_name: &str, kind, line| Edge {
        id: None,
        repo: "web".to_owned(),
        file_id: Some(client),
        from_symbol_id: None,
        to_symbol_id,
        to_name: Some(to_name.to_owned()),
        kind,
        provenance: Provenance::Extracted,
        line,
        col: 10,
        confidence: 0.95,
    };
    db.insert_edges(&[
        edge(Some(session_id), "Session", EdgeKind::References, 1),
        edge(None, "../auth/session", EdgeKind::Imports, 2),
    ])
    .expect("insert edges");

    let sites = get_references(&db, "Session", None).expect("references");

    assert_eq!(
        sites,
        vec![ReferenceSite {
            repo: "web".to_owned(),
            file_path: "src/api/client.ts".to_owned(),
            line: 1,
        }]
    );
}

#[test]
fn test_explore_find_candidates_resolves_a_directory_given_with_its_workspace_prefix() {
    let db = GraphDb::open_in_memory().expect("open in-memory db");
    db.insert_repo("api", "/workspace/api", "main", None)
        .expect("insert repo");
    for (path, name) in [
        ("src/routes/auth.ts", "authRoutes"),
        ("src/routes/admin.ts", "adminRoutes"),
        ("src/lib/noise.ts", "formatBytes"),
    ] {
        let file_id = db
            .upsert_file("api", path, "h", 10, 100)
            .expect("upsert file");
        db.insert_symbols(&[function_symbol(file_id, "api", name, 10)])
            .expect("insert symbol");
    }

    let candidates = find::explore_find_candidates(&db, "services/api/src/routes", None)
        .expect("search candidates");

    let found: std::collections::BTreeSet<&str> =
        candidates.iter().map(|c| c.file_path.as_str()).collect();
    assert_eq!(
        found,
        ["src/routes/admin.ts", "src/routes/auth.ts"]
            .into_iter()
            .collect()
    );
}
