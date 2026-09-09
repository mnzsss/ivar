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
            .contains("2: pub fn init_hall(config: Config) -> Result<Hall> {")
    );
    assert!(snippet.code.contains("4:     setup_logging(&hall);"));
    assert!(snippet.code.contains("6: }"));

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
