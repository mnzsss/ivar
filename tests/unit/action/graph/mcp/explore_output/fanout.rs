use super::*;

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

fn caller_relation(i: usize) -> crate::domain::graph::OperationalRelation {
    crate::domain::graph::OperationalRelation {
        source: crate::domain::graph::RelationEndpoint {
            repo: "api".into(),
            file_path: format!("src/caller_{i:03}.rs"),
            symbol_name: format!("caller_{i:03}"),
            symbol_kind: Some(SymbolKind::Fn),
        },
        target: crate::domain::graph::RelationEndpoint {
            repo: "api".into(),
            file_path: "src/big.rs".into(),
            symbol_name: "run".into(),
            symbol_kind: Some(SymbolKind::Fn),
        },
        direction: crate::domain::graph::RelationDirection::Incoming,
        edge_kind: crate::domain::graph::EdgeKind::Calls,
        provenance: crate::domain::graph::Provenance::Extracted,
        confidence: 1.0,
        line: 5,
        hop_count: 1,
        cross_repo: false,
    }
}

fn big_source(lines: usize) -> crate::domain::graph::SourceFile {
    crate::domain::graph::SourceFile {
        repo: "api".into(),
        file_path: "src/big.rs".into(),
        line_count: lines,
        excerpts: vec![crate::domain::graph::SourceExcerpt {
            start_line: 1,
            end_line: lines,
            code: numbered_code(1, lines, "let value = compute(); "),
        }],
        changed_since_index: false,
    }
}

/// A file with a symbol that has many callers, as `graph_explore paths:[file]` returns it.
fn file_with_many_callers() -> ExploreResult {
    let mut res = empty_result();
    res.query = "src/big.rs".into();
    res.primary_symbols
        .push(primary("run", "src/big.rs", numbered_code(1, 3, "run(); ")));
    res.sources.push(big_source(1_500));
    for i in 0..300 {
        res.call_flows.push(crate::domain::graph::CallFlowItem {
            caller: format!("caller_{i:03}"),
            callee: "run".into(),
            edge_kind: crate::domain::graph::EdgeKind::Calls,
            provenance: crate::domain::graph::Provenance::Extracted,
            line: 5,
        });
        res.direct_relations.push(caller_relation(i));
        res.entry_points.push(caller_relation(i));
    }
    for i in 0..100 {
        res.transitive_consumers
            .push(crate::domain::graph::ExploreImpact {
                symbol_name: format!("consumer_{i:03}"),
                repo: "api".into(),
                file_path: format!("src/consumer_{i:03}.rs"),
                depth: 2,
                path_via: vec!["run".into()],
                cross_repo: false,
            });
    }
    res
}

#[test]
fn many_callers_do_not_remove_the_requested_source() {
    let res = file_with_many_callers();
    for requested in [false, true] {
        let limit = if requested { 24_000 } else { 18_000 };
        let text = render_explore(&res, ExploreFormat::Json, requested).unwrap();
        assert!(text.len() <= limit, "len {}", text.len());
        let value: Value = serde_json::from_str(&text).unwrap();

        assert_eq!(value["output"]["partial"], true);
        let sources = value["sources"].as_array().unwrap();
        assert_eq!(sources.len(), 1, "the requested file must be in sources");
        assert!(value["output"]["omitted"].get("sources").is_none());
        assert_eq!(value["primary_symbols"][0]["symbol"]["name"], "run");

        // Most of the budget is source, and relations only got the remainder.
        let source_bytes = serde_json::to_string(&sources[0]).unwrap().len();
        assert!(source_bytes > limit / 2, "source got {source_bytes} bytes");
        assert!(value["output"]["omitted"]["call_flows"].as_u64().unwrap() > 0);

        let excerpts = sources[0]["excerpts"].as_array().unwrap();
        let end = excerpts.last().unwrap()["end_line"].as_u64().unwrap();
        assert!(end < 1_500);
        assert_eq!(value["output"]["truncated"][0]["section"], "sources");
        assert_eq!(
            value["output"]["next"]["arguments"],
            json!({
                "repo": "api",
                "paths": [format!("src/big.rs:{}-1500", end + 1)],
                "format": "markdown",
            }),
            "next continues after the last returned line and covers the rest"
        );
    }
}

#[test]
fn compact_keeps_symbols_ahead_of_relations_when_callers_overflow() {
    let res = file_with_many_callers();
    let text = render_explore(&res, ExploreFormat::Compact, true).unwrap();
    assert!(text.len() <= 24_000);
    assert!(text.contains("|run|fn|src/big.rs|"), "got: {text}");
    assert!(
        text.contains("direct_relations|"),
        "relations are what gets omitted"
    );
    assert!(!text.contains("primary_symbols|"), "no symbol is omitted");
}
