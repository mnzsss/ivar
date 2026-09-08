//! Integration tests for memory performance, conflict preservation, and backward compatibility.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/integration.rs"]
mod common;

use common::{hall_root, ivar};
use std::time::Instant;

#[test]
fn conflict_preserves_both_versions_as_valid_markdown_files() {
    let (_guard, root) = hall_root();

    ivar().current_dir(&root).arg("init").assert().success();
    ivar()
        .current_dir(&root)
        .args(["memory", "init"])
        .assert()
        .success();

    let scope_dir = root.join("memory/engineering");
    std::fs::create_dir_all(&scope_dir).unwrap();

    let topic_file = scope_dir.join("auth-architecture.md");
    let initial_content = "---\ntitle: Auth Architecture\nscope: engineering\ndescription: Core auth model\ntier: core\nstatus: active\nupdated: '2026-09-08T12:00:00Z'\ntags: [auth]\n---\nInitial auth model\n";
    std::fs::write(&topic_file, initial_content).unwrap();

    // Use ivar binary or internal domain to simulate conflict
    let layout = ivar::store::layout::Layout::at(root.clone());
    let scope = ivar::domain::memory::ScopeName::new("engineering").unwrap();

    let incoming_content = "---\ntitle: Auth Architecture\nscope: engineering\ndescription: Core auth model updated\ntier: core\nstatus: active\nupdated: '2026-09-08T13:00:00Z'\ntags: [auth]\n---\nIncoming conflicting auth model\n";
    let res = ivar::domain::memory::conflict::preserve_topic_conflict(
        &layout,
        &scope,
        "auth-architecture",
        incoming_content,
    )
    .expect("preserve conflict");

    assert!(res.requires_user_action);
    assert_eq!(res.preserved_files.len(), 2);

    let original_path = &res.preserved_files[0];
    let conflict_path = &res.preserved_files[1];

    assert!(original_path.exists());
    assert!(conflict_path.exists());
    assert!(
        conflict_path
            .as_str()
            .contains("auth-architecture.conflict-")
    );
    assert!(conflict_path.as_str().ends_with(".md"));

    assert_eq!(
        std::fs::read_to_string(original_path).unwrap(),
        initial_content
    );
    assert_eq!(
        std::fs::read_to_string(conflict_path).unwrap(),
        incoming_content
    );

    // Verify sync warns about pending conflict (exit code 1 for warnings)
    let sync_assert = ivar().current_dir(&root).arg("sync").assert().code(1);
    let sync_stderr = String::from_utf8(sync_assert.get_output().stderr.clone()).unwrap();
    let sync_stdout = String::from_utf8(sync_assert.get_output().stdout.clone()).unwrap();
    let output = format!("{sync_stdout}\n{sync_stderr}");
    assert!(output.contains("unresolved conflict") || output.contains("conflict"));
}

#[test]
fn performance_bounds_query_p95_under_50ms_and_boot_latency_under_100ms_at_10k_topics() {
    let (_guard, root) = hall_root();

    ivar().current_dir(&root).arg("init").assert().success();
    ivar()
        .current_dir(&root)
        .args(["memory", "init"])
        .assert()
        .success();
    let layout = ivar::store::layout::Layout::at(root.clone());
    let scope_dir = root.join("memory/bench");
    std::fs::create_dir_all(&scope_dir).unwrap();

    // Seed 10,000 topics
    let count = 10_000;
    for i in 0..count {
        let slug = format!("topic_{i:05}");
        let content = format!(
            "---\ntitle: Topic {i}\nscope: bench\ndescription: Benchmark topic item {i}\ntier: core\nstatus: active\nupdated: '2026-09-08T12:00:00Z'\ntags: [bench, t{i}]\n---\nBody of benchmark document number {i} with searchable keyword alpha beta gamma.\n"
        );
        std::fs::write(scope_dir.join(format!("{slug}.md")), content).unwrap();
    }

    // Populate the index
    let index = ivar::store::memory::MemoryIndex::open(&layout).expect("open index");
    index.reconcile(&layout).expect("reconcile index");

    // Test warm local query over 10,000 topics completes in p95 <= 50ms
    let filter = ivar::domain::memory::QueryFilter {
        scope: None,
        limit: 20,
    };

    // Warm-up query
    let _ = index.query("benchmark alpha", &filter).unwrap();

    let mut durations = Vec::with_capacity(100);
    for i in 0..100 {
        let query_str = if i % 2 == 0 {
            "alpha beta"
        } else {
            "keyword gamma"
        };
        let start = Instant::now();
        let matches = index.query(query_str, &filter).expect("query");
        let elapsed = start.elapsed();
        assert!(!matches.is_empty());
        durations.push(elapsed);
    }

    durations.sort();
    let p95 = durations[94];
    assert!(
        p95.as_millis() <= 50,
        "Warm query p95 took {:?} which exceeds 50ms",
        p95
    );

    // Test memory session materialization completes in p95 <= 100ms
    let view_dir = root.join(".ivar/sessions/bench-view");
    std::fs::create_dir_all(&view_dir).unwrap();

    let mut boot_durations = Vec::with_capacity(100);
    let view_path = &view_dir;

    for _ in 0..100 {
        let start = Instant::now();
        ivar::domain::memory::project_memory_symlink(&layout, view_path).expect("project symlink");
        let elapsed = start.elapsed();
        boot_durations.push(elapsed);
    }

    boot_durations.sort();
    let boot_p95 = boot_durations[94];
    assert!(
        boot_p95.as_millis() <= 100,
        "Session materialization p95 took {:?} which exceeds 100ms",
        boot_p95
    );
}

#[test]
fn compatibility_of_existing_commands() {
    let (_guard, root) = hall_root();

    // Verify init, status, sync work smoothly with memory enabled
    ivar().current_dir(&root).arg("init").assert().success();
    ivar()
        .current_dir(&root)
        .args(["memory", "init"])
        .assert()
        .success();
    ivar().current_dir(&root).arg("status").assert().success();
    ivar().current_dir(&root).arg("sync").assert().success();
    ivar()
        .current_dir(&root)
        .args(["status", "--json"])
        .assert()
        .success();
    ivar()
        .current_dir(&root)
        .args(["sync", "--json"])
        .assert()
        .success();
}
