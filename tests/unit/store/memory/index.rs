#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use camino::Utf8PathBuf;
use tempfile::tempdir;

use crate::domain::memory::config::ScopeName;
use crate::domain::memory::query::QueryFilter;
use crate::domain::memory::topic::{
    MemoryTier, MemoryTopic, TopicMetadata, TopicStatus,
};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::memory::document::{delete_topic, write_topic};
use crate::store::memory::index::MemoryIndex;

#[test]
fn index_reconciles_canonical_documents_and_executes_fts5_queries() {
    let dir = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let layout = Layout::at(root);

    let scope = ScopeName::new("architecture").unwrap();
    let topic = MemoryTopic {
        metadata: TopicMetadata {
            title: "Storage Engine".into(),
            scope: scope.clone(),
            description: "SQLite WAL derived index engine".into(),
            tier: MemoryTier::Core,
            status: TopicStatus::Active,
            updated: "2026-09-08T12:00:00Z".into(),
            tags: vec!["sqlite".into(), "fts5".into()],
        },
        content: "# Storage Engine\n\nUses SQLite FTS5 for fast full-text search across all canonical topic documents.".into(),
    };

    write_topic(&layout, &scope, "storage-engine", &topic).expect("write failed");

    let index = MemoryIndex::open(&layout).expect("open index failed");
    let summary = index.reconcile(&layout).expect("reconcile failed");
    assert_eq!(summary.indexed, 1);

    let filter = QueryFilter {
        scope: None,
        limit: 10,
    };
    let results = index.query("FTS5 full-text", &filter).expect("query failed");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Storage Engine");
    assert!(results[0].snippet.contains("full-text"));
}

#[test]
fn index_incremental_reconciliation_detects_modifications_and_deletions() {
    let dir = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let layout = Layout::at(root);

    let scope = ScopeName::new("design").unwrap();
    let topic1 = MemoryTopic {
        metadata: TopicMetadata {
            title: "Color Palette".into(),
            scope: scope.clone(),
            description: "Brand colors".into(),
            tier: MemoryTier::Core,
            status: TopicStatus::Active,
            updated: "2026-09-08T12:00:00Z".into(),
            tags: vec!["design".into()],
        },
        content: "Primary color is sapphire blue and secondary is emerald green.".into(),
    };
    let topic2 = MemoryTopic {
        metadata: TopicMetadata {
            title: "Typography".into(),
            scope: scope.clone(),
            description: "Font hierarchy".into(),
            tier: MemoryTier::Extended,
            status: TopicStatus::Active,
            updated: "2026-09-08T12:00:00Z".into(),
            tags: vec!["fonts".into()],
        },
        content: "Inter is used for body and Berkeley Mono for code snippets.".into(),
    };

    write_topic(&layout, &scope, "colors", &topic1).unwrap();
    write_topic(&layout, &scope, "typography", &topic2).unwrap();

    let index = MemoryIndex::open(&layout).unwrap();
    let summary1 = index.reconcile(&layout).unwrap();
    assert_eq!(summary1.indexed, 2);
    assert_eq!(summary1.updated, 0);
    assert_eq!(summary1.removed, 0);

    // No-op reconcile
    let summary_noop = index.reconcile(&layout).unwrap();
    assert_eq!(summary_noop.indexed, 0);
    assert_eq!(summary_noop.updated, 0);
    assert_eq!(summary_noop.removed, 0);

    // Modify one topic
    let mut updated_topic1 = topic1;
    updated_topic1.content = "Primary color is midnight navy and secondary is neon green.".into();
    write_topic(&layout, &scope, "colors", &updated_topic1).unwrap();

    let summary2 = index.reconcile(&layout).unwrap();
    assert_eq!(summary2.indexed, 0);
    assert_eq!(summary2.updated, 1);
    assert_eq!(summary2.removed, 0);

    let filter = QueryFilter::default();
    let results = index.query("midnight navy", &filter).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Color Palette");

    // Delete one topic
    delete_topic(&layout, &scope, "typography").unwrap();

    let summary3 = index.reconcile(&layout).unwrap();
    assert_eq!(summary3.indexed, 0);
    assert_eq!(summary3.updated, 0);
    assert_eq!(summary3.removed, 1);

    let results_deleted = index.query("Berkeley Mono", &filter).unwrap();
    assert!(results_deleted.is_empty());
}

#[test]
fn index_query_filters_by_scope_and_respects_limit() {
    let dir = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let layout = Layout::at(root);

    let scope_a = ScopeName::new("scope-a").unwrap();
    let scope_b = ScopeName::new("scope-b").unwrap();

    for i in 1..=5 {
        let topic = MemoryTopic {
            metadata: TopicMetadata {
                title: format!("Doc A {i}"),
                scope: scope_a.clone(),
                description: "Shared keyword documentation".into(),
                tier: MemoryTier::Core,
                status: TopicStatus::Active,
                updated: "2026-09-08T12:00:00Z".into(),
                tags: vec![],
            },
            content: format!("Shared search term in topic {i} of scope A"),
        };
        write_topic(&layout, &scope_a, &format!("doc-a-{i}"), &topic).unwrap();
    }

    for i in 1..=3 {
        let topic = MemoryTopic {
            metadata: TopicMetadata {
                title: format!("Doc B {i}"),
                scope: scope_b.clone(),
                description: "Shared keyword documentation".into(),
                tier: MemoryTier::Core,
                status: TopicStatus::Active,
                updated: "2026-09-08T12:00:00Z".into(),
                tags: vec![],
            },
            content: format!("Shared search term in topic {i} of scope B"),
        };
        write_topic(&layout, &scope_b, &format!("doc-b-{i}"), &topic).unwrap();
    }

    let index = MemoryIndex::open(&layout).unwrap();
    index.reconcile(&layout).unwrap();

    // Query with limit
    let filter_limit = QueryFilter {
        scope: None,
        limit: 3,
    };
    let results_limited = index.query("Shared search term", &filter_limit).unwrap();
    assert_eq!(results_limited.len(), 3);

    // Query with scope filter
    let filter_scope_b = QueryFilter {
        scope: Some(scope_b.clone()),
        limit: 10,
    };
    let results_scope_b = index.query("Shared search term", &filter_scope_b).unwrap();
    assert_eq!(results_scope_b.len(), 3);
    for match_item in results_scope_b {
        assert_eq!(match_item.scope, scope_b);
    }
}

#[test]
fn index_recovers_transparently_from_corruption_or_rebuild() {
    let dir = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let layout = Layout::at(root);

    let scope = ScopeName::new("backend").unwrap();
    let topic = MemoryTopic {
        metadata: TopicMetadata {
            title: "Database Engine".into(),
            scope: scope.clone(),
            description: "Postgres and SQLite".into(),
            tier: MemoryTier::Core,
            status: TopicStatus::Active,
            updated: "2026-09-08T12:00:00Z".into(),
            tags: vec!["db".into()],
        },
        content: "Relational database engine with ACID guarantees.".into(),
    };
    write_topic(&layout, &scope, "db-engine", &topic).unwrap();

    let index = MemoryIndex::open(&layout).unwrap();
    index.reconcile(&layout).unwrap();

    // Rebuild index
    let rebuild_summary = index.rebuild(&layout).unwrap();
    assert_eq!(rebuild_summary.indexed, 1);

    // Corrupt database file intentionally
    let db_path = layout.memory_index_db();
    fs::write_bytes(&db_path, b"corrupted non-sqlite file content").unwrap();

    // Reconcile recovers transparently from corrupt DB
    let recovered_summary = index.reconcile(&layout).unwrap();
    assert_eq!(recovered_summary.indexed, 1);

    let results = index
        .query(
            "ACID guarantees",
            &QueryFilter {
                scope: None,
                limit: 10,
            },
        )
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Database Engine");
}

#[test]
fn index_handles_empty_or_special_punctuation_queries() {
    let dir = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let layout = Layout::at(root);

    let scope = ScopeName::new("special").unwrap();
    let topic = MemoryTopic {
        metadata: TopicMetadata {
            title: "Special Punctuation".into(),
            scope: scope.clone(),
            description: "Testing symbols: colons, dashes, etc.".into(),
            tier: MemoryTier::Core,
            status: TopicStatus::Active,
            updated: "2026-09-08T12:00:00Z".into(),
            tags: vec![],
        },
        content: "Testing key:value and c++ and a/b expressions.".into(),
    };
    write_topic(&layout, &scope, "special", &topic).unwrap();

    let index = MemoryIndex::open(&layout).unwrap();
    index.reconcile(&layout).unwrap();

    let filter = QueryFilter::default();

    // Empty query
    assert_eq!(index.query("", &filter).unwrap().len(), 0);
    assert_eq!(index.query("   ", &filter).unwrap().len(), 0);

    // Queries with punctuation that could break naive FTS5 queries
    let results = index.query("c++ key:value", &filter).unwrap();
    assert_eq!(results.len(), 1);
}
