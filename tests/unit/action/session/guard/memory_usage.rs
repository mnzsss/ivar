//! memory_query stamping and memory read misses from hook payloads.

use super::usage::session_env_in_hall;
use super::*;
use crate::store::memory::db::MemoryDb;

fn hall() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_with_promoted_feature();
    drop(MemoryDb::open(Layout::at(root.clone()).memory_db().as_std_path()).unwrap());
    (guard, root)
}

fn misses(root: &Utf8PathBuf) -> Vec<(String, Option<i64>, Option<String>)> {
    let db = MemoryDb::open(Layout::at(root.clone()).memory_db().as_std_path()).unwrap();
    let mut stmt = db
        .conn()
        .prepare("SELECT kind, usage_id, query FROM memory_misses ORDER BY id")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn request(
    tool: &str,
    input: serde_json::Value,
    targets: &[&str],
    writes: bool,
) -> crate::domain::guard::ToolRequest {
    crate::domain::guard::ToolRequest {
        tool: tool.into(),
        targets: targets.iter().map(Utf8PathBuf::from).collect(),
        writes,
        search_pattern: None,
        input,
        agent: None,
        call_id: None,
    }
}

#[test]
fn memory_query_tool_names_are_recognised_for_every_provider_spelling() {
    for name in [
        "memory_query",
        "mcp__acme-ivar__memory_query",
        "acme-ivar_memory_query",
        "acme_ivar_memory_query",
    ] {
        assert!(is_memory_query_tool(name), "{name}");
    }
    assert!(!is_memory_query_tool("memory_query_extra"));
    assert!(!is_memory_query_tool("graph_explore"));
}

#[test]
fn reads_under_the_memory_dirs_are_detected_and_writes_are_not() {
    let cwd = Utf8PathBuf::from("/hall");
    let read = request(
        "Read",
        serde_json::json!({}),
        &["/hall/docs/memory/gotcha/a.md"],
        false,
    );
    let grep = request(
        "Grep",
        serde_json::json!({"pattern": "x", "path": "docs/memory"}),
        &[],
        false,
    );
    let from_docs = request(
        "Read",
        serde_json::json!({}),
        &["memory/decision/b.md"],
        false,
    );
    let write = request(
        "Write",
        serde_json::json!({}),
        &["/hall/docs/memory/gotcha/a.md"],
        true,
    );
    let code = request("Read", serde_json::json!({}), &["/hall/src/a.rs"], false);
    let db_file = request(
        "Read",
        serde_json::json!({}),
        &["/hall/.ivar/memory.db"],
        false,
    );
    let sibling = request(
        "Read",
        serde_json::json!({}),
        &["/hall/docs/memory-notes.md"],
        false,
    );

    assert_eq!(
        memory_read_pattern(&read, &cwd).as_deref(),
        Some("/hall/docs/memory/gotcha/a.md")
    );
    assert_eq!(
        memory_read_pattern(&grep, &cwd).as_deref(),
        Some("/hall/docs/memory")
    );
    assert_eq!(
        memory_read_pattern(&from_docs, &Utf8PathBuf::from("/hall/docs")).as_deref(),
        Some("/hall/docs/memory/decision/b.md")
    );
    assert_eq!(memory_read_pattern(&write, &cwd), None);
    assert_eq!(memory_read_pattern(&code, &cwd), None);
    assert_eq!(memory_read_pattern(&db_file, &cwd), None);
    assert_eq!(memory_read_pattern(&sibling, &cwd), None);
    for lookalike in ["/hall/mydocs/memory/a.md", "/hall/x.ivar/memory/a.md"] {
        let request = request("Read", serde_json::json!({}), &[lookalike], false);
        assert_eq!(memory_read_pattern(&request, &cwd), None, "{lookalike}");
    }
}

#[test]
fn a_read_without_a_query_is_skipped_and_a_read_after_one_is_a_single_followup() {
    let (_guard, root) = hall();
    record_memory_read_at(&root, None, Some("s1".into()), "docs/memory/a.md");
    record_memory_call_at(&root, None, Some("s1".into()), Some("teardown"));
    record_memory_read_at(&root, None, Some("s1".into()), "docs/memory/a.md");
    record_memory_read_at(&root, None, Some("s1".into()), "docs/memory/b.md");

    assert_eq!(
        misses(&root),
        vec![
            ("skipped".to_owned(), None, None),
            ("followup".to_owned(), Some(1), Some("teardown".to_owned())),
        ]
    );
}

#[test]
fn a_read_after_the_window_closes_is_skipped() {
    let (_guard, root) = hall();
    record_memory_call_at(&root, None, Some("s1".into()), Some("teardown"));
    let db = MemoryDb::open(Layout::at(root.clone()).memory_db().as_std_path()).unwrap();
    db.conn()
        .execute(
            "UPDATE memory_usage SET ts = ts - ?1",
            [FOLLOWUP_WINDOW_SECS + 1],
        )
        .unwrap();
    drop(db);

    record_memory_read_at(&root, None, Some("s1".into()), "docs/memory/a.md");

    assert_eq!(misses(&root), vec![("skipped".to_owned(), None, None)]);
}

#[test]
fn a_busy_memory_database_drops_the_miss_within_the_busy_timeout() {
    let (_guard, root) = hall();
    let holder =
        rusqlite::Connection::open(Layout::at(root.clone()).memory_db().as_std_path()).unwrap();
    holder.execute_batch("BEGIN EXCLUSIVE").unwrap();

    let started = std::time::Instant::now();
    record_memory_read_at(&root, None, Some("s1".into()), "docs/memory/a.md");

    assert!(
        started.elapsed() < std::time::Duration::from_millis(500),
        "{:?}",
        started.elapsed()
    );
    holder.execute_batch("ROLLBACK").unwrap();
    assert!(misses(&root).is_empty());
}

#[test]
fn guard_decision_for_a_memory_read_is_unchanged_when_recording_fails() {
    let (_guard, root) = hall_with_promoted_feature();
    let env = session_env_in_hall(&root, "6f0c9d5f-0000-4000-8000-0000000007ff");
    let _ = std::fs::remove_file(Layout::at(root.clone()).memory_db());
    let req = request(
        "Read",
        serde_json::Value::Null,
        &[root.join("docs/memory/gotcha/a.md").as_str()],
        false,
    );

    record_memory_read_at(&env.view_dir, Some(&env), None, "docs/memory/gotcha/a.md");

    let set = resolve_writable_set(&env).unwrap();
    assert!(matches!(
        decide(&Resolution::Resolved(&set), &req, &[]),
        GuardDecision::Allow
    ));
}

#[test]
fn recording_without_a_memory_database_is_a_silent_no_op() {
    let (_guard, root) = hall_with_promoted_feature();
    let _ = std::fs::remove_file(Layout::at(root.clone()).memory_db());
    record_memory_call_at(&root, None, Some("s1".into()), Some("q"));
    record_memory_read_at(&root, None, Some("s1".into()), "docs/memory/a.md");
    assert!(!Layout::at(root).memory_db().exists());
}

fn searching(
    tool: &str,
    input: serde_json::Value,
    pattern: &str,
) -> crate::domain::guard::ToolRequest {
    crate::domain::guard::ToolRequest {
        search_pattern: Some(pattern.into()),
        ..request(tool, input, &[], false)
    }
}

#[test]
fn a_grep_pattern_naming_memory_is_not_a_memory_read() {
    let grep = searching(
        "Grep",
        serde_json::json!({"pattern": "docs/memory/", "path": "src"}),
        "docs/memory/",
    );
    assert_eq!(
        memory_read_pattern(&grep, &Utf8PathBuf::from("/hall")),
        None
    );
}

#[test]
fn a_pathless_grep_from_a_memory_dir_reads_the_cwd() {
    let grep = searching("Grep", serde_json::json!({"pattern": "x"}), "x");
    assert_eq!(
        memory_read_pattern(&grep, &Utf8PathBuf::from("/hall/docs/memory")).as_deref(),
        Some("/hall/docs/memory")
    );
    assert_eq!(
        memory_read_pattern(&grep, &Utf8PathBuf::from("/hall")),
        None
    );
}

#[test]
fn a_bash_search_reaching_memory_is_still_a_memory_read() {
    let input = serde_json::json!({"command": "rg foo /hall/docs/memory"});
    let pattern = crate::providers::extract_search_pattern("Bash", &input);
    let bash = crate::domain::guard::ToolRequest {
        search_pattern: pattern,
        ..request("Bash", input, &[], false)
    };
    assert!(memory_read_pattern(&bash, &Utf8PathBuf::from("/hall")).is_some());
}
