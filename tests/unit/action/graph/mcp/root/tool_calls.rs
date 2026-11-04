use super::*;

#[test]
fn callers_callees_impact_and_outline_default_to_markdown_and_keep_json_on_request() {
    let (db, temp) = setup_test_mcp_db();
    let root = temp.path();

    let (callers, _) = call_tool(&db, root, "get_callers", json!({"symbol": "helper"}));
    assert!(
        callers.starts_with("**Callers of `helper`: 1**"),
        "got: {callers}"
    );
    let (callees, _) = call_tool(&db, root, "get_callees", json!({"symbol": "execute"}));
    assert!(
        callees.starts_with("**Calls made by `execute`: 1**"),
        "got: {callees}"
    );
    let (impact, _) = call_tool(&db, root, "get_impact", json!({"symbol": "helper"}));
    assert!(impact.starts_with("**Impact of `helper`"), "got: {impact}");
    let (outline, _) = call_tool(
        &db,
        root,
        "get_file_outline",
        json!({"file": "src/main.rs", "repo": "my-repo"}),
    );
    assert!(
        outline.starts_with("**Outline of `src/main.rs` (my-repo)**"),
        "got: {outline}"
    );

    let (callers_json, _) = call_tool(
        &db,
        root,
        "get_callers",
        json!({"symbol": "helper", "format": "json"}),
    );
    let parsed: Value = serde_json::from_str(&callers_json).expect("json on request");
    assert_eq!(parsed[0]["caller"]["name"], "execute");
}

#[test]
fn argument_names_agents_guess_are_accepted() {
    let (db, temp) = setup_test_mcp_db();
    let root = temp.path();

    let (explore, explore_failed) =
        call_tool(&db, root, "graph_explore", json!({"symbol": "execute"}));
    assert!(!explore_failed, "got: {explore}");
    assert!(explore.contains("**Exploration: execute**"));

    let (outline, outline_failed) = call_tool(
        &db,
        root,
        "get_file_outline",
        json!({"file_path": "my-repo/src/main.rs"}),
    );
    assert!(!outline_failed, "got: {outline}");
    assert!(outline.contains("`execute`"), "got: {outline}");
}

#[test]
fn unknown_symbols_and_files_answer_with_guidance_instead_of_an_error() {
    let (db, temp) = setup_test_mcp_db();
    let root = temp.path();

    for (tool, args) in [
        ("get_impact", json!({"symbol": "missing_symbol"})),
        ("get_callees", json!({"symbol": "missing_symbol"})),
        ("get_file_outline", json!({"file": "src/missing.rs"})),
    ] {
        let (text, is_error) = call_tool(&db, root, tool, args);
        assert!(
            !is_error,
            "{tool} must not teach the agent to give up: {text}"
        );
        assert!(
            text.contains("graph_explore"),
            "{tool} must name the next call: {text}"
        );
    }
}

#[test]
fn test_mcp_tool_call_explore() {
    let (db, temp) = setup_test_mcp_db();

    let input = format!(
        "{}\n",
        json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/call",
            "params": {
                "name": "graph_explore",
                "arguments": {
                    "query": "execute"
                }
            }
        })
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
    let text = String::from_utf8(output).expect("utf8");
    let resp: Value = serde_json::from_str(text.trim()).expect("parse resp");
    assert_eq!(resp["id"], 10);

    // The default is decision-oriented Markdown: over MCP the consumer is a model
    // picking what to read next, and it ignored the serialised struct.
    let rendered = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert!(rendered.contains("**Exploration: execute**"));
    assert!(rendered.contains("execute"));
    assert!(rendered.contains("helper"));

    // `format: "json"` still returns the structure, for callers that parse it.
    let json_input = format!(
        "{}\n",
        json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "tools/call",
            "params": {
                "name": "graph_explore",
                "arguments": { "query": "execute", "format": "json" }
            }
        })
    );
    let mut json_output = Vec::new();
    run_mcp_server(
        &db,
        Some(temp.path()),
        Cursor::new(json_input),
        &mut json_output,
        |_| Ok(json!({"status": "ok"})),
    )
    .expect("run server");
    let json_resp: Value =
        serde_json::from_str(String::from_utf8(json_output).expect("utf8").trim())
            .expect("parse resp");
    let explore_val: Value = serde_json::from_str(
        json_resp["result"]["content"][0]["text"]
            .as_str()
            .expect("text content"),
    )
    .expect("json parse explore");
    assert_eq!(explore_val["query"], "execute");
    assert!(explore_val["direct_relations"].is_array());
    assert!(explore_val["entry_points"].is_array());
    assert!(explore_val["transitive_consumers"].is_array());

    // Explore compact format
    let compact_input = format!(
        "{}\n",
        json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "tools/call",
            "params": {
                "name": "graph_explore",
                "arguments": {
                    "query": "execute",
                    "format": "compact"
                }
            }
        })
    );
    let mut compact_out = Vec::new();
    run_mcp_server(
        &db,
        Some(temp.path()),
        Cursor::new(compact_input),
        &mut compact_out,
        |_| Ok(json!({"status": "ok"})),
    )
    .expect("run server compact");
    let compact_text = String::from_utf8(compact_out).expect("utf8");
    let compact_resp: Value =
        serde_json::from_str(compact_text.trim()).expect("parse compact resp");
    let compact_content = compact_resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(compact_content.starts_with("#SCHEMA: id|name|kind|file|line|col|complexity"));
    assert!(compact_content.contains("execute"));
    assert!(compact_content.contains("#SCHEMA: source_symbol|source_repo|source_file"));
}

#[test]
fn test_mcp_tool_call_affected_and_path() {
    let (db, temp) = setup_test_mcp_db();

    let input = format!(
        "{}\n{}\n",
        json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "tools/call",
            "params": {
                "name": "get_affected_tests",
                "arguments": {
                    "files": ["src/main.rs"]
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "tools/call",
            "params": {
                "name": "get_path",
                "arguments": {
                    "from": "test_exec",
                    "to": "helper"
                }
            }
        })
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

    let affected_resp: Value = serde_json::from_str(&lines[0]).expect("parse affected");
    assert_eq!(affected_resp["id"], 20);
    let aff_text = affected_resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(aff_text.contains("tests/exec_test.rs"));

    let path_resp: Value = serde_json::from_str(&lines[1]).expect("parse path");
    assert_eq!(path_resp["id"], 21);
    let path_text = path_resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(path_text.contains("test_exec"));
    assert!(path_text.contains("helper"));
}

#[test]
fn test_mcp_malformed_and_unknown_method() {
    let (db, temp) = setup_test_mcp_db();

    let input = "not a json line\n{\"jsonrpc\": \"2.0\", \"id\": 99, \"method\": \"unknown/method\", \"params\": {}}\n";

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

    let err1: Value = serde_json::from_str(&lines[0]).expect("parse err1");
    assert_eq!(err1["error"]["code"], -32700); // Parse error

    let err2: Value = serde_json::from_str(&lines[1]).expect("parse err2");
    assert_eq!(err2["id"], 99);
    assert_eq!(err2["error"]["code"], -32601); // Method not found
}
