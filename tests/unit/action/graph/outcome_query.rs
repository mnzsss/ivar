#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use super::*;
use crate::domain::graph::{GraphStats, MissKind, MissRecord, UsageEvent, UsageSource};
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
    assert!(
        String::from_utf8(empty)
            .unwrap()
            .contains("Usage: none recorded")
    );

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
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Usage:"), "got: {text}");
    assert!(text.contains("explore"), "got: {text}");
    assert!(text.contains("mcp"), "got: {text}");
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
