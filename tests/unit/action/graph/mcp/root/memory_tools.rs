use super::*;

fn rpc(root: &std::path::Path, surface: ToolSurface, request: Value) -> Value {
    let db = GraphDb::open_in_memory().unwrap();
    let cwd = camino::Utf8PathBuf::from_path_buf(root.to_path_buf()).unwrap();
    handle_json_rpc_at(
        &db,
        Some(root),
        &cwd,
        surface,
        &request,
        &mut |_| Ok(json!({"status": "ok"})),
        None,
    )
    .expect("response")
}

#[test]
fn the_default_surface_lists_the_memory_tools_after_the_graph_ones() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let resp = rpc(
        root.as_std_path(),
        ToolSurface::Explore,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    );
    let names: Vec<&str> = resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert_eq!(names[..2], ["graph_explore", "graph_feedback"]);
    for name in ["memory_query", "memory_status", "memory_read_page"] {
        assert!(names.contains(&name), "{name}");
    }
}

#[test]
fn a_memory_call_is_answered_from_the_hall_and_writes_no_usage_row() {
    let (_guard, root) = crate::test_support::seeded_hall();
    std::fs::create_dir_all(root.join("docs/memory/gotcha")).unwrap();
    std::fs::write(
        root.join("docs/memory/gotcha/a.md"),
        "# A\nabout teardown\n",
    )
    .unwrap();
    let db = GraphDb::open_in_memory().unwrap();
    let cwd = root.clone();
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "memory_query", "arguments": {"query": "teardown"}}});

    let resp = handle_json_rpc_at(
        &db,
        Some(root.as_std_path()),
        &cwd,
        ToolSurface::Explore,
        &request,
        &mut |_| Ok(json!({"status": "ok"})),
        None,
    )
    .unwrap();

    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("docs/memory/gotcha/a.md"), "{text}");
    let rows: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM usage", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);
    let memory = crate::store::memory::db::MemoryDb::open(
        crate::store::layout::Layout::at(root.clone())
            .memory_db()
            .as_std_path(),
    )
    .unwrap();
    let source: String = memory
        .conn()
        .query_row("SELECT source FROM memory_usage", [], |r| r.get(0))
        .unwrap();
    assert_eq!(source, "mcp");
}

#[test]
fn the_server_instructions_carry_the_memory_section() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let resp = rpc(
        root.as_std_path(),
        ToolSurface::Explore,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
    );
    let text = resp["result"]["instructions"].as_str().unwrap();
    assert!(text.contains("graph_feedback"));
    assert!(text.contains("memory_query"));
    assert!(text.contains("never `session`"));
    assert!(text.contains(crate::action::memory::mcp::KIND_DEFINITIONS));
}

#[test]
fn memory_lint_returns_the_same_issues_as_the_function_and_the_surface_has_twelve_tools() {
    let (_guard, root) = crate::test_support::seeded_hall();
    std::fs::create_dir_all(root.join("docs/memory/decision")).unwrap();
    std::fs::write(
        root.join("docs/memory/decision/a.md"),
        "---\nconfidence: high\n---\n",
    )
    .unwrap();
    let layout = crate::store::layout::Layout::at(root.clone());

    let text = crate::action::memory::mcp::call(&layout, "memory_lint", &json!({})).unwrap();
    let issues: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(issues["issues"][0]["code"], "lint.field_unknown");
    assert_eq!(issues["issues"][0]["severity"], "error");

    let listed = rpc(
        root.as_std_path(),
        ToolSurface::Explore,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    );
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 12);
}
