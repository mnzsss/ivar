#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use super::*;
use crate::domain::graph::{GraphStats, MissKind, MissRecord, UsageEvent, UsageSource, UsageStats};
use crate::error::WriteHuman;
use crate::store::graph::db::GraphDb;

#[test]
fn relative_age_uses_the_largest_whole_unit() {
    assert_eq!(relative_age(1_000, 958), "42s ago");
    assert_eq!(relative_age(1_000, 700), "5m ago");
    assert_eq!(relative_age(20_000, 9_200), "3h ago");
    assert_eq!(relative_age(200_000, 27_200), "2d ago");
    assert_eq!(relative_age(0, 10), "0s ago");
}

fn usage(source: UsageSource) -> UsageStats {
    UsageStats {
        command: "explore".to_owned(),
        source,
        count: 2,
        last_used: 1_700_000_000,
        empty_count: 0,
        error_count: 0,
        p50_ms: 1,
        p95_ms: 2,
    }
}

#[test]
fn compact_stats_print_the_real_empty_count_for_mcp_rows_too() {
    let outcome = StatsOutcome(GraphStats {
        repo_count: 0,
        file_count: 0,
        symbol_count: 0,
        edge_count: 0,
        db_size_bytes: 0,
        layers: Vec::new(),
        usage: vec![
            UsageStats {
                empty_count: 3,
                ..usage(UsageSource::Mcp)
            },
            usage(UsageSource::Cli),
        ],
    });
    let compact = outcome.to_compact();
    assert!(
        compact.contains("\nexplore|mcp|2|1700000000|3|0|1|2"),
        "{compact}"
    );
    assert!(
        compact.contains("\nexplore|cli|2|1700000000|0|0|1|2"),
        "{compact}"
    );
}

#[test]
fn stats_human_output_lists_usage_or_says_none() {
    let db = GraphDb::open_in_memory().unwrap();
    let mut empty = Vec::new();
    StatsOutcome(db.stats().unwrap())
        .write_human(&mut empty)
        .unwrap();
    let empty_text = anstream::adapter::strip_str(&String::from_utf8(empty).unwrap()).to_string();
    assert!(empty_text.contains("Usage: none recorded"));

    db.record_usage(&UsageEvent {
        command: "explore".to_owned(),
        source: UsageSource::Mcp,
        duration_ms: 9,
        result_count: None,
        error: false,
        session: None,
        query: None,
    })
    .unwrap();
    let mut out = Vec::new();
    StatsOutcome(db.stats().unwrap())
        .write_human(&mut out)
        .unwrap();
    let text = anstream::adapter::strip_str(&String::from_utf8(out).unwrap()).to_string();
    assert!(text.contains("Usage:\nCOMMAND  SRC  COUNT  EMPTY  ERRORS  P50_MS  P95_MS  LAST_USED\nexplore  mcp      1      0       0       9       9  0s ago\n"), "got: {text}");
}

fn sample_miss(id: i64, ts: i64, kind: MissKind) -> MissRecord {
    MissRecord {
        id,
        ts,
        session: Some("sess-1".to_owned()),
        kind,
        query: Some("get_callers".to_owned()),
        pattern: Some("rg get_callers".to_owned()),
        reason: None,
    }
}

#[test]
fn misses_json_serializes_as_a_plain_array_newest_first() {
    let outcome = MissesOutcome {
        misses: vec![
            sample_miss(2, 200, MissKind::Followup),
            sample_miss(1, 100, MissKind::Skipped),
        ],
    };
    let parsed = serde_json::to_value(&outcome.misses).unwrap();
    let array = parsed
        .as_array()
        .expect("misses JSON must be a plain array");
    assert_eq!(array.len(), 2);
    assert_eq!(array[0]["id"], 2);
    assert_eq!(array[0]["kind"], "followup");
    assert_eq!(array[1]["id"], 1);
    assert_eq!(array[1]["kind"], "skipped");
}

#[test]
fn misses_human_output_lists_kind_and_pattern_or_says_none() {
    let mut empty = Vec::new();
    MissesOutcome { misses: Vec::new() }
        .write_human(&mut empty)
        .unwrap();
    assert!(
        String::from_utf8(empty)
            .unwrap()
            .contains("no misses recorded")
    );

    let mut out = Vec::new();
    MissesOutcome {
        misses: vec![sample_miss(1, 100, MissKind::Skipped)],
    }
    .write_human(&mut out)
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("skipped"), "got: {text}");
    assert!(text.contains("rg get_callers"), "got: {text}");
}

#[test]
fn misses_compact_has_schema_and_one_row_per_miss() {
    let compact = MissesOutcome {
        misses: vec![sample_miss(1, 100, MissKind::Feedback)],
    }
    .to_compact();
    assert_eq!(
        compact,
        "#SCHEMA: id|ts|kind|session|query|pattern|reason\n1|100|feedback|sess-1|get_callers|rg get_callers|"
    );
}

#[test]
fn cli_explore_outcome_caps_direct_relations_at_max_list_items_with_remainder() {
    use crate::action::graph::outcome::ops::ExploreOutcome;
    use crate::domain::graph::{
        EdgeKind, ExploreResult, OperationalRelation, Provenance, RelationDirection,
        RelationEndpoint,
    };

    let direct_relations: Vec<OperationalRelation> = (0..100)
        .map(|i| OperationalRelation {
            source: RelationEndpoint {
                symbol_name: format!("caller_{i}"),
                symbol_kind: None,
                repo: "api".to_owned(),
                file_path: format!("src/caller_{i}.rs"),
            },
            target: RelationEndpoint {
                symbol_name: "target_fn".to_owned(),
                symbol_kind: None,
                repo: "api".to_owned(),
                file_path: "src/target.rs".to_owned(),
            },
            direction: RelationDirection::Incoming,
            edge_kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            confidence: 1.0,
            line: i + 1,
            hop_count: 1,
            cross_repo: false,
        })
        .collect();

    let res = ExploreResult {
        query: "target_fn".to_owned(),
        file_matches: Vec::new(),
        primary_symbols: Vec::new(),
        call_flows: Vec::new(),
        impact_summary: None,
        direct_relations,
        entry_points: Vec::new(),
        transitive_consumers: Vec::new(),
        sources: Vec::new(),
        flows: Vec::new(),
        not_shown: Vec::new(),
    };

    let mut buf = Vec::new();
    ExploreOutcome(res).write_human(&mut buf).unwrap();
    let text = String::from_utf8(buf).unwrap();

    assert!(
        text.contains("caller_0 [api:src/caller_0.rs]"),
        "first item should be printed"
    );
    assert!(
        text.contains("caller_39 [api:src/caller_39.rs]"),
        "40th item should be printed"
    );
    assert!(
        !text.contains("caller_40 [api:src/caller_40.rs]"),
        "41st item must be capped"
    );
    assert!(
        text.contains("…and 60 more"),
        "remainder summary must state omitted item count, got: {text}"
    );
}

#[test]
fn cli_explore_outcome_stops_appending_sections_when_exceeding_max_output_chars() {
    use crate::action::graph::narrate::MAX_OUTPUT_CHARS;
    use crate::action::graph::outcome::ops::ExploreOutcome;
    use crate::domain::graph::{
        ExploreImpact, ExploreResult, Span, Symbol, SymbolKind, SymbolSnippet,
    };

    // Create a large primary symbol source that consumes almost all characters
    let long_code = "x".repeat(MAX_OUTPUT_CHARS + 500);
    let primary_symbols = vec![SymbolSnippet {
        symbol: Symbol {
            id: Some(1),
            file_id: Some(1),
            repo: "api".to_owned(),
            name: "huge_fn".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("pub fn huge_fn()".to_owned()),
            docstring: None,
            span: Span::new(1, 1, 100, 1),
            is_exported: true,
            complexity: None,
        },
        file_path: "src/huge.rs".to_owned(),
        code: long_code,
        start_line: 1,
        end_line: 100,
    }];

    let transitive_consumers = vec![ExploreImpact {
        symbol_name: "consumer_fn".to_owned(),
        repo: "api".to_owned(),
        file_path: "src/consumer.rs".to_owned(),
        depth: 2,
        path_via: vec!["intermediate_fn".to_owned()],
        cross_repo: false,
    }];

    let res = ExploreResult {
        query: "huge_fn".to_owned(),
        file_matches: Vec::new(),
        primary_symbols,
        call_flows: Vec::new(),
        impact_summary: Some("Large impact summary".to_owned()),
        direct_relations: Vec::new(),
        entry_points: Vec::new(),
        transitive_consumers,
        sources: Vec::new(),
        flows: Vec::new(),
        not_shown: Vec::new(),
    };

    let mut buf = Vec::new();
    ExploreOutcome(res).write_human(&mut buf).unwrap();
    let text = String::from_utf8(buf).unwrap();

    assert!(
        !text.contains("Transitive Consumers:"),
        "transitive consumers should be omitted once buffer budget is exceeded"
    );
    assert!(
        !text.contains("Impact Summary:"),
        "impact summary should be omitted once buffer budget is exceeded"
    );
}
