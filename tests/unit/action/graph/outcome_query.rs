#![allow(clippy::unwrap_used)]

use super::*;
use crate::domain::graph::{GraphStats, UsageEvent};
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
fn compact_stats_print_a_dash_for_mcp_empty_counts() {
    let outcome = StatsOutcome(GraphStats {
        repo_count: 0,
        file_count: 0,
        symbol_count: 0,
        edge_count: 0,
        db_size_bytes: 0,
        layers: Vec::new(),
        usage: vec![usage(UsageSource::Mcp), usage(UsageSource::Cli)],
    });
    let compact = outcome.to_compact();
    assert!(
        compact.contains("\nexplore|mcp|2|1700000000|-|0|1|2"),
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
