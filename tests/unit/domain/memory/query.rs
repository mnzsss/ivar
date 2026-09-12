#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::domain::memory::config::ScopeName;
use crate::domain::memory::query::{QueryFilter, QueryMatch, ReconcileSummary};

#[test]
fn query_filter_default_and_scope_filtering() {
    let default_filter = QueryFilter::default();
    assert!(default_filter.scope.is_none());
    assert!(default_filter.limit > 0);

    let scope = ScopeName::new("architecture").unwrap();
    let filter = QueryFilter {
        scope: Some(scope.clone()),
        limit: 5,
    };
    assert_eq!(filter.scope.as_ref(), Some(&scope));
    assert_eq!(filter.limit, 5);
}

#[test]
fn reconcile_summary_default_and_counts() {
    let summary = ReconcileSummary::default();
    assert_eq!(summary.indexed, 0);
    assert_eq!(summary.updated, 0);
    assert_eq!(summary.removed, 0);

    let active_summary = ReconcileSummary {
        indexed: 3,
        updated: 2,
        removed: 1,
    };
    assert_eq!(active_summary.indexed, 3);
    assert_eq!(active_summary.updated, 2);
    assert_eq!(active_summary.removed, 1);
}

#[test]
fn query_match_properties() {
    let scope = ScopeName::new("testing").unwrap();
    let match_item = QueryMatch {
        scope: scope.clone(),
        slug: "fts5-testing".into(),
        path: "memory/testing/fts5-testing.md".into(),
        title: "FTS5 Testing".into(),
        snippet: "Testing full-text search".into(),
        rank: -1.23,
    };

    assert_eq!(match_item.scope, scope);
    assert_eq!(match_item.slug, "fts5-testing");
    assert_eq!(match_item.title, "FTS5 Testing");
}
