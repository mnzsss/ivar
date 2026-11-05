use super::*;

#[test]
fn test_mcp_initialize_and_tools_list() {
    let (db, temp) = setup_test_mcp_db();

    let input = format!(
        "{}\n{}\n",
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
    );

    let mut output = Vec::new();
    run_mcp_server(
        &db,
        Some(temp.path()),
        Cursor::new(input),
        &mut output,
        |_| Ok(json!({"status": "ok"})),
    )
    .expect("run server");
    let lines: Vec<String> = String::from_utf8(output)
        .expect("utf8")
        .lines()
        .map(|s| s.to_owned())
        .collect();

    assert_eq!(lines.len(), 2);

    let init_resp: Value = serde_json::from_str(&lines[0]).expect("parse init");
    assert_eq!(init_resp["id"], 1);
    assert_eq!(init_resp["result"]["serverInfo"]["name"], "ivar");

    let list_resp: Value = serde_json::from_str(&lines[1]).expect("parse list");
    assert_eq!(list_resp["id"], 2);
    let tools = list_resp["result"]["tools"]
        .as_array()
        .expect("tools array");
    assert_eq!(tools.len(), 21);
    let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(tool_names.contains(&"graph_feedback"));
    assert!(tool_names.contains(&"graph_explore"));
    assert!(tool_names.contains(&"get_callers"));
    assert!(tool_names.contains(&"get_callees"));
    assert!(tool_names.contains(&"get_file_outline"));
    assert!(tool_names.contains(&"get_affected_tests"));
    assert!(tool_names.contains(&"get_path"));
    assert!(tool_names.contains(&"get_impact"));
    assert!(tool_names.contains(&"refresh_index"));
    assert!(tool_names.contains(&"get_dead_code"));
    assert!(tool_names.contains(&"get_complexity"));
    assert!(tool_names.contains(&"get_hierarchy"));
    assert!(tool_names.contains(&"memory_query"));
    assert!(tool_names.contains(&"memory_recent"));
    assert!(tool_names.contains(&"memory_read_session_observations"));
    assert!(tool_names.contains(&"memory_write_page"));
    assert!(tool_names.contains(&"memory_patch_page"));
    assert!(tool_names.contains(&"memory_delete_page"));
    assert!(tool_names.contains(&"memory_lint"));
    assert!(
        !tool_names.contains(&"get_graph_stats"),
        "get_graph_stats stays out of the tool list"
    );

    let explore = tools
        .iter()
        .find(|t| t["name"] == "graph_explore")
        .expect("graph_explore");
    assert_eq!(explore["_meta"]["anthropic/alwaysLoad"], true);
    assert_eq!(explore["annotations"]["readOnlyHint"], true);
}

#[test]
fn the_default_tool_surface_lists_graph_explore_and_graph_feedback() {
    let tools = super::tools::list_tools(super::tools::ToolSurface::default());
    let names: Vec<&str> = tools
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(names[..2], ["graph_explore", "graph_feedback"]);
}

#[test]
fn symbol_tools_take_the_names_agents_guess_and_guide_a_missing_symbol() {
    let (db, temp) = setup_test_mcp_db();
    let root = temp.path();

    for args in [json!({"query": "helper"}), json!({"name": "helper"})] {
        let (callers, failed) = call_tool(&db, root, "get_callers", args);
        assert!(!failed, "got: {callers}");
        assert!(
            callers.starts_with("**Callers of `helper`: 1**"),
            "got: {callers}"
        );
    }
    for tool in ["get_callers", "get_callees", "get_impact", "get_hierarchy"] {
        let (text, failed) = call_tool(&db, root, tool, json!({}));
        assert!(!failed, "{tool}: {text}");
        assert!(text.contains("`symbol`"), "{tool}: {text}");
    }
}

#[test]
fn explore_outline_and_symbol_schemas_leave_argument_names_to_the_server() {
    let tools = super::tools::list_tools(super::tools::ToolSurface::All);
    for name in [
        "graph_explore",
        "get_file_outline",
        "get_callers",
        "get_hierarchy",
    ] {
        let tool = tools
            .as_array()
            .expect("tools array")
            .iter()
            .find(|t| t["name"] == name)
            .expect("tool listed");
        assert!(
            tool["inputSchema"].get("required").is_none(),
            "a host that validates `required` rejects aliases before dispatch sees them: {name}"
        );
    }
}

#[test]
fn graph_explore_takes_a_path_argument_and_answers_a_missing_query_with_guidance() {
    let (db, temp) = setup_test_mcp_db();
    let root = temp.path();

    for args in [
        json!({"path": "src/main.rs"}),
        json!({"intent": "execute"}),
        json!({"paths": ["src/main.rs"]}),
        json!({"queries": ["execute", "helper"]}),
    ] {
        let (explore, failed) = call_tool(&db, root, "graph_explore", args.clone());
        assert!(!failed, "{args}: {explore}");
        assert!(explore.contains("`execute`"), "{args}: {explore}");
    }

    let (missing, missing_failed) = call_tool(&db, root, "graph_explore", json!({}));
    assert!(!missing_failed, "got: {missing}");
    assert!(missing.contains("`query`"), "got: {missing}");
}

#[test]
fn graph_explore_joins_every_argument_an_agent_splits_its_query_across() {
    let (db, temp) = setup_test_mcp_db();
    let root = temp.path();

    for args in [
        json!({"intent": "helper", "paths": ["tests/exec_test.rs"]}),
        json!({"symbols": ["helper"], "files": ["tests/exec_test.rs"]}),
    ] {
        let (explore, failed) = call_tool(&db, root, "graph_explore", args.clone());
        assert!(!failed, "{args}: {explore}");
        assert!(
            explore.starts_with("**Exploration: helper tests/exec_test.rs**"),
            "{args}: {explore}"
        );
        assert!(
            explore.contains("`tests/exec_test.rs` ("),
            "the named file's source is part of the answer: {args}: {explore}"
        );
    }
}

#[test]
fn a_query_listing_file_paths_returns_those_files_whole_and_names_unindexed_ones() {
    let (db, temp) = setup_test_mcp_db();
    let root = temp.path();
    index_file(&db, root, "src/bulky.rs", &long_file(), "bulky");

    let (answer, failed) = call_tool(
        &db,
        root,
        "graph_explore",
        json!({"query": "my-repo/src/bulky.rs src/main.rs tests/exec_test.rs verify.mjs"}),
    );

    assert!(!failed, "{answer}");
    assert!(
        answer.contains("// line 300"),
        "whole file expected: {answer}"
    );
    assert!(answer.contains("fn test_exec()"), "{answer}");
    assert!(answer.contains("pub fn helper()"), "{answer}");
    assert!(
        answer.contains("Not indexed: `verify.mjs`"),
        "unresolved path is named: {answer}"
    );
}

#[test]
fn a_query_mixing_words_and_paths_keeps_intent_behavior() {
    let (db, temp) = setup_test_mcp_db();
    let root = temp.path();
    index_file(&db, root, "src/bulky.rs", &long_file(), "bulky");

    let (answer, failed) = call_tool(
        &db,
        root,
        "graph_explore",
        json!({"query": "bulky src/bulky.rs"}),
    );

    assert!(!failed, "{answer}");
    assert!(answer.contains("`bulky`"), "{answer}");
    assert!(!answer.contains("// line 300"), "intent slices: {answer}");
    assert!(!answer.contains("Not indexed"), "{answer}");
}
