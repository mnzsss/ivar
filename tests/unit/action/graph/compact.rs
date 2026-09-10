//! Unit tests for compact pipe-delimited encoding format.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::graph::query::{
    CalleeInfo, CallerInfo, ImpactItem, ImpactResult, SymbolLocation,
};
use crate::domain::graph::{
    AffectedResult, ComplexityItem, DeadCodeItem, EdgeKind, ExploreResult, HierarchyItem,
    PathResult, PathStep, Provenance, Span, Symbol, SymbolKind, SymbolSnippet,
};

fn sample_symbol(id: Option<i64>, name: &str, kind: SymbolKind, line: usize, col: usize) -> Symbol {
    Symbol {
        id,
        file_id: Some(1),
        repo: "test_repo".into(),
        name: name.into(),
        kind,
        scope: None,
        signature: None,
        docstring: None,
        span: Span::new(line, col, line + 5, 0),
        is_exported: true,
        complexity: Some(3),
    }
}

#[test]
fn test_encode_symbols_and_trait() {
    let syms = vec![
        SymbolLocation {
            symbol: sample_symbol(Some(101), "fetch_user", SymbolKind::Fn, 12, 4),
            file_path: "src/user.rs".into(),
        },
        SymbolLocation {
            symbol: sample_symbol(Some(102), "UserRecord", SymbolKind::Struct, 50, 0),
            file_path: "src/models.rs".into(),
        },
    ];

    let compact = syms.to_compact();
    let expected = format!(
        "{}\n101|fetch_user|fn|src/user.rs|12|4|3\n102|UserRecord|struct|src/models.rs|50|0|3",
        SYMBOL_SCHEMA
    );
    assert_eq!(compact, expected);

    // Byte/token savings vs serde_json::to_string_pretty
    let json = serde_json::to_string_pretty(&syms).unwrap();
    assert!(
        compact.len() < json.len() / 2,
        "Compact format length ({}) must be < 50% of JSON pretty length ({})",
        compact.len(),
        json.len()
    );
}

#[test]
fn test_encode_dead_code() {
    let items = vec![DeadCodeItem {
        symbol: sample_symbol(Some(1), "unused_helper", SymbolKind::Fn, 42, 0),
        file_path: "src/utils.rs".into(),
        line: 42,
    }];

    let compact = items.to_compact();
    let expected = format!("{}\nunused_helper|fn|src/utils.rs|42", DEAD_CODE_SCHEMA);
    assert_eq!(compact, expected);

    let json = serde_json::to_string_pretty(&items).unwrap();
    assert!(compact.len() < json.len() / 2);
}

#[test]
fn test_encode_complexity() {
    let items = vec![ComplexityItem {
        symbol: sample_symbol(Some(1), "complex_func", SymbolKind::Fn, 10, 0),
        file_path: "src/engine.rs".into(),
        complexity: 15,
        line: 10,
    }];

    let compact = items.to_compact();
    let expected = format!("{}\n15|complex_func|fn|src/engine.rs|10", COMPLEXITY_SCHEMA);
    assert_eq!(compact, expected);

    let json = serde_json::to_string_pretty(&items).unwrap();
    assert!(compact.len() < json.len() / 2);
}

#[test]
fn test_encode_hierarchy() {
    let item = HierarchyItem {
        symbol: sample_symbol(Some(1), "Dog", SymbolKind::Struct, 1, 0),
        file_path: "src/animals.rs".into(),
        bases: vec!["Animal".into(), "LivingThing".into()],
        implementations: vec!["Bark".into(), "Walk".into()],
    };

    let compact = item.to_compact();
    let expected = format!(
        "{}\nDog|struct|src/animals.rs|Animal,LivingThing|Bark,Walk",
        HIERARCHY_SCHEMA
    );
    assert_eq!(compact, expected);

    let none_compact = (None as Option<&HierarchyItem>).to_compact();
    assert_eq!(none_compact, HIERARCHY_SCHEMA);

    let json = serde_json::to_string_pretty(&item).unwrap();
    assert!(compact.len() < json.len() / 2);
}

#[test]
fn test_encode_callers() {
    let callers = vec![CallerInfo {
        caller: sample_symbol(Some(1), "main", SymbolKind::Fn, 5, 0),
        caller_file_path: "src/main.rs".into(),
        edge_kind: EdgeKind::Calls,
        provenance: Provenance::Extracted,
        confidence: 1.0,
        line: 8,
        col: 4,
    }];

    let compact = callers.to_compact();
    let expected = format!("{}\nmain|fn|src/main.rs|8|CALLS|1.00", CALLER_SCHEMA);
    assert_eq!(compact, expected);

    let json = serde_json::to_string_pretty(&callers).unwrap();
    assert!(compact.len() < json.len() / 2);
}

#[test]
fn test_encode_callees() {
    let callees = vec![CalleeInfo {
        callee_name: "helper".into(),
        callee_symbol: Some(sample_symbol(Some(2), "helper", SymbolKind::Fn, 20, 0)),
        callee_file_path: Some("src/lib.rs".into()),
        edge_kind: EdgeKind::Calls,
        provenance: Provenance::Extracted,
        confidence: 0.95,
        line: 12,
        col: 2,
    }];

    let compact = callees.to_compact();
    let expected = format!("{}\nhelper|fn|src/lib.rs|12|CALLS|0.95", CALLEE_SCHEMA);
    assert_eq!(compact, expected);

    let json = serde_json::to_string_pretty(&callees).unwrap();
    assert!(compact.len() < json.len() / 2);
}

#[test]
fn test_encode_impact() {
    let impact = ImpactResult {
        root_symbol: sample_symbol(Some(1), "root_fn", SymbolKind::Fn, 1, 0),
        affected_symbols: vec![ImpactItem {
            symbol: sample_symbol(Some(2), "downstream", SymbolKind::Fn, 10, 0),
            file_path: "src/downstream.rs".into(),
            depth: 2,
            path_via: vec!["root_fn".into(), "mid_fn".into(), "downstream".into()],
        }],
        affected_files: vec!["src/downstream.rs".into()],
        total_affected: 1,
    };

    let compact = impact.to_compact();
    let expected = format!(
        "{}\ndownstream|fn|src/downstream.rs|2|root_fn,mid_fn,downstream",
        IMPACT_SCHEMA
    );
    assert_eq!(compact, expected);

    let json = serde_json::to_string_pretty(&impact).unwrap();
    assert!(compact.len() < json.len() / 2);
}

#[test]
fn test_encode_affected() {
    let affected = AffectedResult {
        changed_files: vec!["src/lib.rs".into()],
        affected_test_files: vec!["tests/test_lib.rs".into(), "tests/integration.rs".into()],
        recommendations: vec![crate::domain::graph::AffectedRecommendation {
            repo: "ivar".into(),
            test_file: "tests/test_lib.rs".into(),
            causal_path: vec![crate::domain::graph::CausalStep {
                source: "tests/test_lib.rs".into(),
                target: "src/lib.rs".into(),
                edge_kind: EdgeKind::Imports,
                provenance: Provenance::Extracted,
                confidence: 1.0,
                line: 1,
            }],
            direct_change: false,
            hop_count: 1,
            edge_kind: EdgeKind::Imports,
            provenance: Provenance::Extracted,
            confidence: 1.0,
            reason: "imports src/lib.rs (1 hop)".into(),
            command: Some("cargo test --test test_lib".into()),
        }],
    };

    let compact = affected.to_compact();
    let expected = format!(
        "{}\ntests/test_lib.rs\ntests/integration.rs\n{}\nivar|tests/test_lib.rs|false|1|imports|extracted|1.00|cargo test --test test_lib|imports src/lib.rs (1 hop)|tests/test_lib.rs -[imports]-> src/lib.rs",
        AFFECTED_SCHEMA, AFFECTED_RECOMMENDATION_SCHEMA
    );
    assert_eq!(compact, expected);

    let json = serde_json::to_string_pretty(&affected).unwrap();
    assert!(compact.len() < json.len() / 2);
}

#[test]
fn test_encode_path() {
    let path = PathResult {
        from: "A".into(),
        to: "C".into(),
        steps: vec![
            PathStep {
                source: "A".into(),
                target: "B".into(),
                edge_kind: EdgeKind::Calls,
                line: 10,
            },
            PathStep {
                source: "B".into(),
                target: "C".into(),
                edge_kind: EdgeKind::Calls,
                line: 25,
            },
        ],
    };

    let compact = path.to_compact();
    let expected = format!("{}\n1|B|||CALLS\n2|C|||CALLS", PATH_SCHEMA);
    assert_eq!(compact, expected);

    let json = serde_json::to_string_pretty(&path).unwrap();
    assert!(compact.len() < json.len() / 2);
}

#[test]
fn test_encode_explore() {
    let explore = ExploreResult {
        query: "fetch".into(),
        primary_symbols: vec![SymbolSnippet {
            symbol: sample_symbol(Some(10), "fetch", SymbolKind::Fn, 1, 0),
            file_path: "src/net.rs".into(),
            code: "fn fetch() {}".into(),
            start_line: 1,
            end_line: 1,
        }],
        call_flows: vec![],
        impact_summary: None,
        sources: vec![],
        direct_relations: vec![crate::domain::graph::OperationalRelation {
            source: crate::domain::graph::RelationEndpoint {
                repo: "ivar".into(),
                file_path: "src/main.rs".into(),
                symbol_name: "main".into(),
                symbol_kind: Some(SymbolKind::Fn),
            },
            target: crate::domain::graph::RelationEndpoint {
                repo: "ivar".into(),
                file_path: "src/net.rs".into(),
                symbol_name: "fetch".into(),
                symbol_kind: Some(SymbolKind::Fn),
            },
            direction: crate::domain::graph::RelationDirection::Incoming,
            edge_kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            confidence: 1.0,
            line: 42,
            hop_count: 1,
            cross_repo: false,
        }],
        entry_points: vec![],
        transitive_consumers: vec![crate::domain::graph::ExploreImpact {
            symbol_name: "caller_func".into(),
            repo: "ivar-orca".into(),
            file_path: "src/index.ts".into(),
            depth: 2,
            path_via: vec!["main".into(), "caller_func".into()],
            cross_repo: true,
        }],
    };

    let compact = explore.to_compact();
    let expected = format!(
        "{}\n10|fetch|fn|src/net.rs|1|0|3\n{}\nmain|ivar|src/main.rs|incoming|fetch|ivar|src/net.rs|calls|extracted|1.00|42|1|false\n{}\ncaller_func|ivar-orca|src/index.ts|2|main -> caller_func|true",
        SYMBOL_SCHEMA, RELATION_SCHEMA, EXPLORE_IMPACT_SCHEMA
    );
    assert_eq!(compact, expected);

    let json = serde_json::to_string_pretty(&explore).unwrap();
    assert!(compact.len() < json.len() / 2);
}
