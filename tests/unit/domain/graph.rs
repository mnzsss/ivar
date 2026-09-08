#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn test_symbol_kinds_and_construction() {
    let kinds = [
        SymbolKind::Fn,
        SymbolKind::Method,
        SymbolKind::Struct,
        SymbolKind::Class,
        SymbolKind::Trait,
        SymbolKind::Interface,
        SymbolKind::Enum,
        SymbolKind::Mod,
        SymbolKind::Const,
        SymbolKind::Other("type_alias".to_owned()),
    ];

    for kind in kinds {
        let sym = Symbol {
            id: Some(1),
            file_id: Some(42),
            repo: "ivar".to_owned(),
            name: "test_sym".to_owned(),
            kind: kind.clone(),
            scope: Some("crate::domain".to_owned()),
            signature: Some("fn test_sym() -> ()".to_owned()),
            docstring: Some("A doc comment".to_owned()),
            span: Span::new(10, 1, 20, 1),
            is_exported: true,
            complexity: Some(3),
        };
        assert_eq!(sym.kind, kind);
        assert_eq!(sym.complexity, Some(3));
        assert!(sym.is_exported);
        assert_eq!(sym.span.start_line, 10);
        assert_eq!(sym.span.end_line, 20);
    }
}

#[test]
fn test_edge_kinds_provenance_and_construction() {
    let kinds = [
        EdgeKind::Calls,
        EdgeKind::Imports,
        EdgeKind::Implements,
        EdgeKind::Inherits,
        EdgeKind::CrossImports,
        EdgeKind::CrossExecutes,
        EdgeKind::CrossCallsHttp,
        EdgeKind::Other("dynamic_dispatch".to_owned()),
    ];

    let provenances = [
        Provenance::Extracted,
        Provenance::Inferred,
        Provenance::Ambiguous,
    ];

    for kind in kinds {
        for &prov in &provenances {
            let edge = Edge {
                id: Some(10),
                repo: "ivar".to_owned(),
                file_id: Some(42),
                from_symbol_id: Some(1),
                to_symbol_id: Some(2),
                to_name: Some("target_fn".to_owned()),
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
        query: "Symbol".to_owned(),
        primary_symbols: vec![SymbolSnippet {
            symbol: Symbol {
                id: Some(1),
                file_id: Some(1),
                repo: "ivar".to_owned(),
                name: "Symbol".to_owned(),
                kind: SymbolKind::Struct,
                scope: None,
                signature: Some("pub struct Symbol".to_owned()),
                docstring: None,
                span: Span::new(1, 1, 10, 1),
                is_exported: true,
                complexity: None,
            },
            file_path: "src/domain/graph.rs".to_owned(),
            code: "pub struct Symbol { ... }".to_owned(),
            start_line: 1,
            end_line: 10,
        }],
        call_flows: vec![CallFlowItem {
            caller: "main".to_owned(),
            callee: "init".to_owned(),
            edge_kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 42,
        }],
        impact_summary: Some("Core domain model".to_owned()),
    };

    let json = serde_json::to_string(&explore).expect("serialize explore");
    let deserialized: ExploreResult = serde_json::from_str(&json).expect("deserialize explore");
    assert_eq!(explore, deserialized);
}

#[test]
fn test_affected_result_json_roundtrip() {
    let affected = AffectedResult {
        changed_files: vec!["src/domain/graph.rs".to_owned()],
        affected_test_files: vec!["tests/graph_test.rs".to_owned()],
    };

    let json = serde_json::to_string(&affected).expect("serialize affected");
    let deserialized: AffectedResult = serde_json::from_str(&json).expect("deserialize affected");
    assert_eq!(affected, deserialized);
}

#[test]
fn test_path_result_json_roundtrip() {
    let path = PathResult {
        from: "main".to_owned(),
        to: "execute".to_owned(),
        steps: vec![PathStep {
            source: "main".to_owned(),
            target: "execute".to_owned(),
            edge_kind: EdgeKind::Calls,
            line: 55,
        }],
    };

    let json = serde_json::to_string(&path).expect("serialize path");
    let deserialized: PathResult = serde_json::from_str(&json).expect("deserialize path");
    assert_eq!(path, deserialized);
}

#[test]
fn test_analysis_items_json_roundtrip() {
    let dead_code = DeadCodeItem {
        symbol: Symbol {
            id: Some(1),
            file_id: Some(1),
            repo: "ivar".to_owned(),
            name: "unused_func".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn unused_func()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: false,
            complexity: Some(1),
        },
        file_path: "src/unused.rs".to_owned(),
        line: 1,
    };
    let json = serde_json::to_string(&dead_code).expect("serialize dead_code");
    let deserialized: DeadCodeItem = serde_json::from_str(&json).expect("deserialize dead_code");
    assert_eq!(dead_code, deserialized);

    let complexity_item = ComplexityItem {
        symbol: Symbol {
            id: Some(2),
            file_id: Some(1),
            repo: "ivar".to_owned(),
            name: "complex_func".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("fn complex_func()".to_owned()),
            docstring: None,
            span: Span::new(10, 1, 30, 1),
            is_exported: true,
            complexity: Some(15),
        },
        file_path: "src/complex.rs".to_owned(),
        complexity: 15,
        line: 10,
    };
    let json = serde_json::to_string(&complexity_item).expect("serialize complexity_item");
    let deserialized: ComplexityItem =
        serde_json::from_str(&json).expect("deserialize complexity_item");
    assert_eq!(complexity_item, deserialized);

    let hierarchy_item = HierarchyItem {
        symbol: Symbol {
            id: Some(3),
            file_id: Some(2),
            repo: "ivar".to_owned(),
            name: "MyClass".to_owned(),
            kind: SymbolKind::Class,
            scope: None,
            signature: Some("class MyClass extends BaseClass".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 20, 1),
            is_exported: true,
            complexity: None,
        },
        file_path: "src/class.ts".to_owned(),
        bases: vec!["BaseClass".to_owned()],
        implementations: vec!["InterfaceA".to_owned(), "InterfaceB".to_owned()],
    };
    let json = serde_json::to_string(&hierarchy_item).expect("serialize hierarchy_item");
    let deserialized: HierarchyItem =
        serde_json::from_str(&json).expect("deserialize hierarchy_item");
    assert_eq!(hierarchy_item, deserialized);
}
