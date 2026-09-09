use crate::action::graph::view::{
    MAX_DEPTH, MAX_NODES, ViewError, ViewSeed, ViewerEdge, ViewerGraph, ViewerNode,
    collect_subgraph, expand_node, get_node_details, search_symbols, query_path, query_impact,
};
use crate::domain::graph::{Edge, EdgeKind, Provenance, Span, Symbol, SymbolKind};
use crate::store::graph::db::GraphDb;

fn seed_test_graph(db: &GraphDb) -> (i64, i64, i64) {
    db.insert_repo("repo_a", "/path/to/repo_a", "main", None).expect("insert repo");
    let file_id = db
        .upsert_file("repo_a", "src/lib.rs", "hash1", 0, 0)
        .expect("upsert file");

    let sym1 = Symbol {
        id: None,
        file_id: Some(file_id),
        repo: "repo_a".to_string(),
        name: "alpha".to_string(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("fn alpha()".to_string()),
        docstring: None,
        span: Span::new(1, 0, 5, 0),
        is_exported: true,
        complexity: Some(2),
    };
    let sym2 = Symbol {
        id: None,
        file_id: Some(file_id),
        repo: "repo_a".to_string(),
        name: "beta".to_string(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("fn beta()".to_string()),
        docstring: None,
        span: Span::new(6, 0, 10, 0),
        is_exported: false,
        complexity: Some(1),
    };
    let sym3 = Symbol {
        id: None,
        file_id: Some(file_id),
        repo: "repo_a".to_string(),
        name: "gamma".to_string(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("fn gamma()".to_string()),
        docstring: None,
        span: Span::new(11, 0, 15, 0),
        is_exported: false,
        complexity: Some(4),
    };

    let ids = db.insert_symbols(&[sym1, sym2, sym3]).expect("insert symbols");
    let s1 = ids[0];
    let s2 = ids[1];
    let s3 = ids[2];

    let e1 = Edge {
        id: None,
        repo: "repo_a".to_string(),
        file_id: Some(file_id),
        from_symbol_id: Some(s1),
        to_symbol_id: Some(s2),
        to_name: Some("beta".to_string()),
        kind: EdgeKind::Calls,
        provenance: Provenance::Extracted,
        line: 2,
        col: 4,
        confidence: 1.0,
    };
    let e2 = Edge {
        id: None,
        repo: "repo_a".to_string(),
        file_id: Some(file_id),
        from_symbol_id: Some(s2),
        to_symbol_id: Some(s3),
        to_name: Some("gamma".to_string()),
        kind: EdgeKind::Calls,
        provenance: Provenance::Ambiguous,
        line: 7,
        col: 4,
        confidence: 0.7,
    };
    db.insert_edges(&[e1, e2]).expect("insert edges");

    (s1, s2, s3)
}

#[test]
fn test_collect_subgraph_deterministic_bounds_and_provenance() {
    let db = GraphDb::open_in_memory().expect("open db");
    let (s1, s2, s3) = seed_test_graph(&db);

    // Query from seed symbol "alpha" with depth 1
    let graph = collect_subgraph(&db, &ViewSeed::Symbol("alpha".to_string()), 1, 10)
        .expect("collect subgraph");

    assert_eq!(graph.depth, 1);
    assert!(!graph.truncated);
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.nodes[0].name, "alpha");
    assert_eq!(graph.nodes[1].name, "beta");
    assert_eq!(graph.edges[0].from, s1);
    assert_eq!(graph.edges[0].to, s2);
    assert_eq!(graph.edges[0].provenance, Provenance::Extracted);

    // Query from seed symbol "alpha" with depth 2
    let graph2 = collect_subgraph(&db, &ViewSeed::Symbol("alpha".to_string()), 2, 10)
        .expect("collect subgraph");
    assert_eq!(graph2.nodes.len(), 3);
    assert_eq!(graph2.edges.len(), 2);
    assert_eq!(graph2.edges[1].provenance, Provenance::Ambiguous);

    // Query with limit 2 truncates deterministically
    let graph_capped = collect_subgraph(&db, &ViewSeed::Symbol("alpha".to_string()), 2, 2)
        .expect("collect capped");
    assert_eq!(graph_capped.nodes.len(), 2);
    assert!(graph_capped.truncated);
    // Edge to s3 omitted because endpoint s3 was truncated
    assert_eq!(graph_capped.edges.len(), 1);
}

#[test]
fn test_expand_node_and_node_details() {
    let db = GraphDb::open_in_memory().expect("open db");
    let (s1, s2, _s3) = seed_test_graph(&db);

    let details = get_node_details(&db, s2).expect("node details");
    assert_eq!(details.node.name, "beta");
    assert_eq!(details.callers.len(), 1);
    assert_eq!(details.callers[0].caller.name, "alpha");
    assert_eq!(details.callees.len(), 1);
    assert_eq!(details.callees[0].callee_name, "gamma");

    let expansion = expand_node(&db, s1, 10).expect("expand node");
    assert!(expansion.nodes.iter().any(|n| n.id == s2));
}

#[test]
fn test_depth_and_node_limits_enforced() {
    let db = GraphDb::open_in_memory().expect("open db");
    seed_test_graph(&db);

    // Depth > MAX_DEPTH is clamped or errors
    let err = collect_subgraph(&db, &ViewSeed::Default, MAX_DEPTH + 1, 100);
    assert!(matches!(err, Err(ViewError::InvalidParam(_))));

    // Limit > MAX_NODES is clamped or errors
    let err2 = collect_subgraph(&db, &ViewSeed::Default, 1, MAX_NODES + 1);
    assert!(matches!(err2, Err(ViewError::InvalidParam(_))));
}
