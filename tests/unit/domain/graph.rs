#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn test_symbol_kinds_and_construction() {
    let kinds = vec![
        SymbolKind::Fn,
        SymbolKind::Method,
        SymbolKind::Struct,
        SymbolKind::Class,
        SymbolKind::Trait,
        SymbolKind::Interface,
        SymbolKind::Enum,
        SymbolKind::Mod,
        SymbolKind::Const,
        SymbolKind::Other("type_alias".to_string()),
    ];

    for kind in kinds {
        let sym = Symbol {
            id: Some(1),
            file_id: Some(42),
            repo: "ivar".to_string(),
            name: "test_sym".to_string(),
            kind: kind.clone(),
            scope: Some("crate::domain".to_string()),
            signature: Some("fn test_sym() -> ()".to_string()),
            docstring: Some("A doc comment".to_string()),
            span: Span::new(10, 1, 20, 1),
            is_exported: true,
        };
        assert_eq!(sym.kind, kind);
        assert!(sym.is_exported);
        assert_eq!(sym.span.start_line, 10);
        assert_eq!(sym.span.end_line, 20);
    }
}

#[test]
fn test_edge_kinds_provenance_and_construction() {
    let kinds = vec![
        EdgeKind::Calls,
        EdgeKind::Imports,
        EdgeKind::Implements,
        EdgeKind::CrossImports,
        EdgeKind::CrossExecutes,
        EdgeKind::CrossCallsHttp,
        EdgeKind::Other("dynamic_dispatch".to_string()),
    ];

    let provenances = vec![
        Provenance::Extracted,
        Provenance::Inferred,
        Provenance::Ambiguous,
    ];

    for kind in kinds {
        for &prov in &provenances {
            let edge = Edge {
                id: Some(10),
                repo: "ivar".to_string(),
                file_id: Some(42),
                from_symbol_id: Some(1),
                to_symbol_id: Some(2),
                to_name: Some("target_fn".to_string()),
                kind: kind.clone(),
                provenance: prov,
                line: 15,
                col: 5,
                confidence: 0.95,
            };
            assert_eq!(edge.kind, kind);
            assert_eq!(edge.provenance, prov);
            assert!((edge.confidence - 0.95).abs() < f64::EPSILON);
        }
    }
}

#[test]
fn test_graph_stats_json_roundtrip() {
    let stats = GraphStats {
        repo_count: 3,
        file_count: 120,
        symbol_count: 1500,
        edge_count: 4200,
        db_size_bytes: 1048576,
    };

    let json = serde_json::to_string(&stats).expect("serialize stats");
    let deserialized: GraphStats = serde_json::from_str(&json).expect("deserialize stats");
    assert_eq!(stats, deserialized);
}

#[test]
fn test_explore_result_json_roundtrip() {
    let explore = ExploreResult {
        query: "Symbol".to_string(),
        primary_symbols: vec![SymbolSnippet {
            symbol: Symbol {
                id: Some(1),
                file_id: Some(1),
                repo: "ivar".to_string(),
                name: "Symbol".to_string(),
                kind: SymbolKind::Struct,
                scope: None,
                signature: Some("pub struct Symbol".to_string()),
                docstring: None,
                span: Span::new(1, 1, 10, 1),
                is_exported: true,
            },
            file_path: "src/domain/graph.rs".to_string(),
            code: "pub struct Symbol { ... }".to_string(),
            start_line: 1,
            end_line: 10,
        }],
        call_flows: vec![CallFlowItem {
            caller: "main".to_string(),
            callee: "init".to_string(),
            edge_kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 42,
        }],
        impact_summary: Some("Core domain model".to_string()),
    };

    let json = serde_json::to_string(&explore).expect("serialize explore");
    let deserialized: ExploreResult = serde_json::from_str(&json).expect("deserialize explore");
    assert_eq!(explore, deserialized);
}

#[test]
fn test_affected_result_json_roundtrip() {
    let affected = AffectedResult {
        changed_files: vec!["src/domain/graph.rs".to_string()],
        affected_test_files: vec!["tests/graph_test.rs".to_string()],
    };

    let json = serde_json::to_string(&affected).expect("serialize affected");
    let deserialized: AffectedResult = serde_json::from_str(&json).expect("deserialize affected");
    assert_eq!(affected, deserialized);
}

#[test]
fn test_path_result_json_roundtrip() {
    let path = PathResult {
        from: "main".to_string(),
        to: "execute".to_string(),
        steps: vec![PathStep {
            source: "main".to_string(),
            target: "execute".to_string(),
            edge_kind: EdgeKind::Calls,
            line: 55,
        }],
    };

    let json = serde_json::to_string(&path).expect("serialize path");
    let deserialized: PathResult = serde_json::from_str(&json).expect("deserialize path");
    assert_eq!(path, deserialized);
}
