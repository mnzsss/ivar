#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use super::*;
use crate::domain::graph::{ExploreResult, Span, Symbol, SymbolKind, SymbolSnippet};
use serde_json::{Value, json};

fn empty_result() -> ExploreResult {
    ExploreResult {
        query: String::new(),
        file_matches: vec![],
        primary_symbols: vec![],
        call_flows: vec![],
        impact_summary: None,
        direct_relations: vec![],
        entry_points: vec![],
        transitive_consumers: vec![],
        sources: vec![],
        flows: vec![],
        not_shown: vec![],
    }
}
fn primary(name: &str, path: &str, code: String) -> SymbolSnippet {
    SymbolSnippet {
        symbol: Symbol {
            id: Some(1),
            file_id: Some(1),
            repo: "api".into(),
            name: name.into(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(1, 1, 1, 1),
            is_exported: true,
            complexity: None,
        },
        file_path: path.into(),
        code,
        start_line: 1,
        end_line: 1,
    }
}

#[test]
fn complete_json_at_the_byte_boundary_stays_complete() {
    for requested in [false, true] {
        let limit = if requested { 24_000 } else { 18_000 };
        let base = serde_json::to_string_pretty(&empty_result()).unwrap().len();
        for size in [limit - 1, limit, limit + 1] {
            let mut res = empty_result();
            res.query = "x".repeat(size - base);
            let text = render_explore(&res, ExploreFormat::Json, requested).unwrap();
            assert!(text.len() <= limit);
            assert_eq!(
                text,
                render_explore(&res, ExploreFormat::Json, requested).unwrap(),
                "same result and request must produce identical output"
            );
            let parsed: Value = serde_json::from_str(&text).unwrap();
            if size <= limit {
                assert_eq!(parsed["query"], res.query);
                assert!(parsed.get("output").is_none());
                assert_eq!(text.len(), size);
            } else {
                assert_eq!(parsed["query"], "");
                assert_eq!(parsed["output"]["partial"], true);
                assert!(
                    parsed["output"]["omitted_fields"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|field| field == "query")
                );
                assert!(parsed["output"].get("next").is_none());
            }
        }
    }
}

#[test]
fn complete_compact_at_the_byte_boundary_stays_complete() {
    for requested in [false, true] {
        let limit = if requested { 24_000 } else { 18_000 };
        let mut res = empty_result();
        res.file_matches.push(crate::domain::graph::FileMatch {
            repo: "api".into(),
            file_path: "src/boundary.rs".into(),
            match_kind: crate::domain::graph::FileMatchKind::Content,
            start_line: 1,
            excerpt: String::new(),
            content_truncated: false,
        });
        let base = crate::action::graph::compact::encode_explore(&res).len();
        for size in [limit - 1, limit, limit + 1] {
            res.file_matches[0].excerpt = "x".repeat(size - base);
            let text = render_explore(&res, ExploreFormat::Compact, requested).unwrap();
            assert!(text.len() <= limit);
            assert_eq!(
                text,
                render_explore(&res, ExploreFormat::Compact, requested).unwrap()
            );
            if size <= limit {
                assert_eq!(text.len(), size);
                assert!(!text.contains("#SCHEMA: partial|budget_bytes"));
                let row = text.lines().nth(1).unwrap();
                assert!(row.starts_with("api|src/boundary.rs|content|1|"));
                assert!(row.ends_with("|false"));
            } else {
                assert!(text.contains("file_matches|1"));
                assert!(text.contains(&format!("true|{limit}")));
            }
        }
    }
}

#[test]
fn oversized_first_identity_does_not_starve_a_later_primary_record() {
    let mut res = empty_result();
    res.primary_symbols = vec![
        primary(
            &"💡".repeat(10_000),
            "src/oversized.rs",
            "fn oversized() {}".into(),
        ),
        primary("keep", "src/keep.rs", "fn keep() {}".into()),
    ];
    for requested in [false, true] {
        for format in [ExploreFormat::Compact, ExploreFormat::Json] {
            let text = render_explore(&res, format, requested).unwrap();
            assert!(text.len() <= if requested { 24_000 } else { 18_000 });
            if format == ExploreFormat::Json {
                let value: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(value["primary_symbols"][0]["symbol"]["name"], "keep");
                assert_eq!(value["output"]["omitted"]["primary_symbols"], 1);
                assert_eq!(value["output"]["next"]["arguments"]["repo"], "api");
            } else {
                assert!(text.contains("|keep|fn|src/keep.rs|1|1|"));
                assert!(text.contains("primary_symbols|1"));
                assert!(!text.contains('💡'));
            }
        }
    }
}

#[test]
fn giant_escaped_source_record_is_omitted_whole_with_a_source_request() {
    let mut res = empty_result();
    res.primary_symbols
        .push(primary("large", "src/large.rs", "\"💡\\\n".repeat(10_000)));
    res.primary_symbols
        .push(primary("keep", "src/keep.rs", "fn keep() {}".into()));
    let text = render_explore(&res, ExploreFormat::Json, false).unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    assert!(text.len() <= 18_000);
    assert_eq!(value["primary_symbols"].as_array().unwrap().len(), 1);
    assert_eq!(value["primary_symbols"][0]["symbol"]["name"], "keep");
    assert_eq!(value["output"]["omitted"]["primary_symbols"], 1);
    assert_eq!(
        value["output"]["next"]["arguments"],
        json!({"repo":"api","paths":["src/large.rs:1-1"],"format":"markdown"})
    );
}

#[test]
fn compact_metadata_escapes_pipes_without_cutting_the_next_argument() {
    let mut res = empty_result();
    res.file_matches.push(crate::domain::graph::FileMatch {
        repo: "api".into(),
        file_path: "src/a|b.rs".into(),
        match_kind: crate::domain::graph::FileMatchKind::Content,
        start_line: 2,
        excerpt: "x".repeat(30_000),
        content_truncated: false,
    });
    let text = render_explore(&res, ExploreFormat::Compact, false).unwrap();
    assert!(text.len() <= 18_000);
    let row = text
        .lines()
        .find(|line| line.starts_with("graph_explore|{"))
        .unwrap();
    assert_eq!(row.split('|').count(), 2);
    let next: Value = serde_json::from_str(row.split_once('|').unwrap().1).unwrap();
    assert_eq!(next["paths"], json!(["src/a|b.rs:2-41"]));
    assert!(text.contains("file_matches|1"));
}

#[test]
fn large_omitted_flow_with_matching_endpoint_emits_valid_json_and_derived_continuation() {
    let mut res = empty_result();
    let mut hub_snippet = primary("hub_entry", "src/hub.rs", "fn hub_entry() {}".into());
    hub_snippet.symbol.repo = "core-repo".into();
    res.primary_symbols.push(hub_snippet);
    res.direct_relations
        .push(crate::domain::graph::OperationalRelation {
            source: crate::domain::graph::RelationEndpoint {
                repo: "core-repo".into(),
                file_path: "src/hub.rs".into(),
                symbol_name: "hub_entry".into(),
                symbol_kind: Some(SymbolKind::Fn),
            },
            target: crate::domain::graph::RelationEndpoint {
                repo: "worker-repo".into(),
                file_path: "src/worker_target.rs".into(),
                symbol_name: "worker_target".into(),
                symbol_kind: Some(SymbolKind::Fn),
            },
            direction: crate::domain::graph::RelationDirection::Outgoing,
            edge_kind: crate::domain::graph::EdgeKind::Calls,
            provenance: crate::domain::graph::Provenance::Extracted,
            confidence: 1.0,
            line: 12,
            hop_count: 1,
            cross_repo: true,
        });
    res.flows.push(crate::domain::graph::PathResult {
        from: "hub_entry".into(),
        to: "worker_target".into(),
        steps: (0..300)
            .map(|i| crate::domain::graph::PathStep {
                source: if i == 0 {
                    "hub_entry".into()
                } else {
                    format!("step_{i}")
                },
                target: if i == 299 {
                    "worker_target".into()
                } else {
                    format!("step_{}", i + 1)
                },
                edge_kind: crate::domain::graph::EdgeKind::Calls,
                line: 12,
            })
            .collect(),
    });

    let text = render_explore(&res, ExploreFormat::Json, false).unwrap();
    assert!(text.len() <= 18_000);
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["output"]["partial"], true);
    assert_eq!(value["output"]["budget_bytes"], 18_000);
    assert_eq!(value["primary_symbols"][0]["symbol"]["name"], "hub_entry");
    assert_eq!(value["output"]["omitted"]["flows"], 1);
    assert_eq!(
        value["output"]["next"]["arguments"],
        json!({
            "repo": "core-repo",
            "paths": ["src/hub.rs:12-51"],
            "format": "markdown"
        })
    );
}

#[test]
fn retainable_scalars_and_priority_primary_are_preserved_under_relation_and_consumer_fanout() {
    let mut res = empty_result();
    res.query = "symbol_lookup_query_string".into();
    res.impact_summary = Some("1 primary symbol with downstream impact across services".into());
    res.primary_symbols.push(primary(
        "target_core",
        "src/target.rs",
        "pub fn target_core() {}".into(),
    ));

    for i in 0..150 {
        res.direct_relations
            .push(crate::domain::graph::OperationalRelation {
                source: crate::domain::graph::RelationEndpoint {
                    repo: "service-a".into(),
                    file_path: format!("src/caller_{i:03}.rs"),
                    symbol_name: format!("caller_{i:03}"),
                    symbol_kind: Some(SymbolKind::Fn),
                },
                target: crate::domain::graph::RelationEndpoint {
                    repo: "api".into(),
                    file_path: "src/target.rs".into(),
                    symbol_name: "target_core".into(),
                    symbol_kind: Some(SymbolKind::Fn),
                },
                direction: crate::domain::graph::RelationDirection::Incoming,
                edge_kind: crate::domain::graph::EdgeKind::Calls,
                provenance: crate::domain::graph::Provenance::Extracted,
                confidence: 1.0,
                line: 5,
                hop_count: 1,
                cross_repo: true,
            });
    }

    for i in 0..150 {
        res.transitive_consumers
            .push(crate::domain::graph::ExploreImpact {
                symbol_name: format!("consumer_{i:03}"),
                repo: "service-b".into(),
                file_path: format!("src/consumer_{i:03}.rs"),
                depth: 2,
                path_via: vec!["target_core".into(), format!("caller_{i:03}")],
                cross_repo: true,
            });
    }

    let first_text = render_explore(&res, ExploreFormat::Json, false).unwrap();
    let second_text = render_explore(&res, ExploreFormat::Json, false).unwrap();
    assert_eq!(
        first_text, second_text,
        "render_explore must be strictly deterministic"
    );

    assert!(first_text.len() <= 18_000);
    let value: Value = serde_json::from_str(&first_text).unwrap();
    assert_eq!(value["output"]["partial"], true);
    assert_eq!(value["query"], "symbol_lookup_query_string");
    assert_eq!(
        value["impact_summary"],
        "1 primary symbol with downstream impact across services"
    );
    assert_eq!(value["primary_symbols"].as_array().unwrap().len(), 1);
    assert_eq!(value["primary_symbols"][0]["symbol"]["name"], "target_core");

    let admitted_relations = value["direct_relations"].as_array().unwrap().len();
    let omitted_relations = value["output"]["omitted"]["direct_relations"]
        .as_u64()
        .unwrap() as usize;
    assert_eq!(admitted_relations + omitted_relations, 150);
    assert!(omitted_relations > 0);

    let admitted_consumers = value["transitive_consumers"].as_array().unwrap().len();
    let omitted_consumers = value["output"]["omitted"]["transitive_consumers"]
        .as_u64()
        .unwrap() as usize;
    assert_eq!(admitted_consumers + omitted_consumers, 150);
    assert_eq!(omitted_consumers, 150);
    assert_eq!(admitted_consumers, 0);

    let next = &value["output"]["next"]["arguments"];
    assert_eq!(next["repo"], "service-a");
    assert_eq!(next["format"], "markdown");
    let expected_omitted_file = format!("src/caller_{:03}.rs:5-44", admitted_relations);
    assert_eq!(next["paths"], json!([expected_omitted_file]));
}
