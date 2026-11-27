#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::eval::{run, sample};
use crate::action::Ctx;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;
use crate::store::memory::usage::{QueryRecord, record_query};
use crate::test_support::seeded_hall;

fn page(layout: &Layout, rel: &str, text: &str) {
    let path = layout.root().join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn usage(layout: &Layout, source: &str, query: &str) {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    record_query(
        db.conn(),
        &QueryRecord {
            source: source.to_owned(),
            session: None,
            query: query.to_owned(),
            filters: "{}".to_owned(),
            page_count: 1,
            observation_count: 0,
            duration_ms: 1,
            error: false,
        },
    )
    .unwrap();
}

fn count(layout: &Layout, table: &str) -> i64 {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    db.conn()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn labels(layout: &Layout, lines: &[&str]) -> std::path::PathBuf {
    let file = layout.root().join("labels.jsonl").into_std_path_buf();
    std::fs::write(&file, lines.join("\n")).unwrap();
    file
}

#[test]
fn sample_lists_distinct_non_hook_queries_with_empty_expected() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    usage(&layout, "cli", "teardown");
    usage(&layout, "mcp", "teardown");
    usage(&layout, "hook", "from a hook");
    let out = layout.root().join("sample.jsonl").into_std_path_buf();

    let outcome = sample(&Ctx::new(root), 30, 100, Some(out.clone()))
        .unwrap()
        .value;

    assert_eq!(outcome.queries.len(), 1);
    assert_eq!(outcome.queries[0].query, "teardown");
    let text = std::fs::read_to_string(out).unwrap();
    assert_eq!(text, "{\"query\":\"teardown\",\"expected\":[]}\n");
}

#[test]
fn run_scores_hits_and_misses_without_recording_usage() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    page(&layout, "docs/memory/gotcha/a.md", "# A\nabout teardown\n");
    page(&layout, "docs/memory/gotcha/b.md", "# B\nabout locks\n");
    let file = labels(
        &layout,
        &[
            r#"{"query":"teardown","expected":["docs/memory/gotcha/a.md"]}"#,
            r#"{"query":"teardown","expected":["docs/memory/gotcha/b.md"]}"#,
        ],
    );

    let report = run(&Ctx::new(root), &file, 3).unwrap();

    assert!(report.warnings.is_empty());
    let out = report.value;
    assert_eq!(out.scored, 2);
    assert!(out.queries[0].hit);
    assert!((out.queries[0].reciprocal_rank - 1.0).abs() < f64::EPSILON);
    assert!(!out.queries[1].hit);
    assert!(out.queries[1].recall.abs() < f64::EPSILON);
    assert!((out.aggregate.hit_rate - 0.5).abs() < f64::EPSILON);
    assert_eq!(count(&layout, "memory_usage"), 0);
    assert_eq!(count(&layout, "memory_misses"), 0);
}

#[test]
fn run_skips_unlabelled_lines_and_warns_about_missing_pages() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    page(&layout, "docs/memory/gotcha/a.md", "# A\nabout teardown\n");
    let file = labels(
        &layout,
        &[
            r#"{"query":"teardown","expected":[]}"#,
            "",
            r#"{"query":"teardown","expected":["docs/memory/gotcha/a.md","docs/memory/gotcha/gone.md"]}"#,
            r#"{"query":"teardown","expected":["docs/memory/gotcha/gone.md"]}"#,
        ],
    );

    let report = run(&Ctx::new(root), &file, 3).unwrap();

    assert_eq!(report.warnings.len(), 1);
    let out = report.value;
    assert_eq!(out.skipped_unlabelled, 1);
    assert_eq!(out.skipped_unresolvable, 1);
    assert_eq!(out.scored, 1);
    assert_eq!(out.queries[0].missing, ["docs/memory/gotcha/gone.md"]);
    assert!((out.queries[0].recall - 1.0).abs() < f64::EPSILON);
}
