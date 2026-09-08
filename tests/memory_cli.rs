//! Integration tests for ivar memory CLI commands.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "support/integration.rs"]
mod common;

use common::{hall_root, ivar};

#[test]
fn memory_init_and_query_cli_json_parity() {
    let (_guard, root) = hall_root();

    // Initialize ivar hall
    ivar().current_dir(&root).arg("init").assert().success();

    // Run ivar memory init
    ivar()
        .current_dir(&root)
        .args(["memory", "init", "--json"])
        .assert()
        .success();

    // Verify memory directory created
    assert!(root.join("memory").exists());

    // Create a topic document
    let scope_dir = root.join("memory/tech");
    std::fs::create_dir_all(&scope_dir).unwrap();
    std::fs::write(
        scope_dir.join("engine.md"),
        "---\ntitle: Search Engine\nscope: tech\ndescription: SQLite FTS5\ntier: core\nstatus: active\nupdated: '2026-09-08T12:00:00Z'\ntags: [fts5]\n---\nFull text retrieval engine.\n",
    ).unwrap();

    // Run query and verify actual match output in JSON
    let assert = ivar()
        .current_dir(&root)
        .args(["memory", "query", "retrieval", "--json"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Search Engine"));
    assert!(stdout.contains("tech"));
}

#[test]
fn memory_reindex_and_validate_cli() {
    let (_guard, root) = hall_root();

    ivar().current_dir(&root).arg("init").assert().success();
    ivar().current_dir(&root).args(["memory", "init"]).assert().success();

    let scope_dir = root.join("memory/tech");
    std::fs::create_dir_all(&scope_dir).unwrap();
    std::fs::write(
        scope_dir.join("indexer.md"),
        "---\ntitle: Fast Indexer\nscope: tech\ndescription: Memory indexer\ntier: core\nstatus: active\nupdated: '2026-09-08T12:00:00Z'\ntags: [fast]\n---\nIndexes markdown into sqlite.\n",
    ).unwrap();

    // Reindex
    ivar()
        .current_dir(&root)
        .args(["memory", "reindex", "--force", "--json"])
        .assert()
        .success();

    // Validate
    let assert = ivar()
        .current_dir(&root)
        .args(["memory", "validate", "--json"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("\"valid\":true"));
}
