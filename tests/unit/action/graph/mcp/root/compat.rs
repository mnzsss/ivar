use super::*;

#[test]
fn test_mcp_tool_call_refresh_index_success_and_error() {
    let (db, temp) = setup_test_mcp_db();

    let input = format!(
        "{}\n{}\n",
        json!({
            "jsonrpc": "2.0",
            "id": 30,
            "method": "tools/call",
            "params": {
                "name": "refresh_index",
                "arguments": {
                    "repo": "my-repo"
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 31,
            "method": "tools/call",
            "params": {
                "name": "refresh_index",
                "arguments": {}
            }
        })
    );

    let mut seen_repos = Vec::new();
    let mut output = Vec::new();
    run_mcp_server(
        &db,
        Some(temp.path()),
        Cursor::new(input),
        &mut output,
        |repo| {
            seen_repos.push(repo.map(str::to_owned));
            if repo == Some("my-repo") {
                Ok(json!({"status": "indexed", "repo": "my-repo", "files": 2}))
            } else {
                Err("lock acquisition failed".to_owned())
            }
        },
    )
    .expect("run server");

    assert_eq!(seen_repos, vec![Some("my-repo".to_owned()), None]);

    let lines: Vec<String> = String::from_utf8(output)
        .expect("utf8")
        .lines()
        .map(|s| s.to_owned())
        .collect();

    assert_eq!(lines.len(), 2);

    let success_resp: Value = serde_json::from_str(&lines[0]).expect("parse success_resp");
    assert_eq!(success_resp["id"], 30);
    let success_text = success_resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let success_payload: Value = serde_json::from_str(success_text).expect("parse payload");
    assert_eq!(success_payload["status"], "indexed");
    assert_eq!(success_payload["files"], 2);
    assert!(!success_resp["result"]["isError"].as_bool().unwrap_or(false));

    let err_resp: Value = serde_json::from_str(&lines[1]).expect("parse err_resp");
    assert_eq!(err_resp["id"], 31);
    assert_eq!(err_resp["result"]["isError"], true);
    let err_text = err_resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(err_text.contains("lock acquisition failed"));
}

#[test]
fn test_mcp_tool_call_dead_code_complexity_hierarchy_and_compact() {
    let (db, temp) = setup_test_mcp_db();

    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n",
        json!({"jsonrpc": "2.0", "id": 100, "method": "tools/call", "params": {"name": "get_dead_code", "arguments": {"repo": "my-repo", "limit": 10}}}),
        json!({"jsonrpc": "2.0", "id": 101, "method": "tools/call", "params": {"name": "get_dead_code", "arguments": {"repo": "my-repo", "limit": 10, "format": "compact"}}}),
        json!({"jsonrpc": "2.0", "id": 102, "method": "tools/call", "params": {"name": "get_complexity", "arguments": {"repo": "my-repo", "threshold": 1}}}),
        json!({"jsonrpc": "2.0", "id": 103, "method": "tools/call", "params": {"name": "get_complexity", "arguments": {"repo": "my-repo", "threshold": 1, "format": "compact"}}}),
        json!({"jsonrpc": "2.0", "id": 104, "method": "tools/call", "params": {"name": "get_hierarchy", "arguments": {"symbol": "execute", "repo": "my-repo"}}}),
        json!({"jsonrpc": "2.0", "id": 105, "method": "tools/call", "params": {"name": "get_hierarchy", "arguments": {"symbol": "execute", "repo": "my-repo", "format": "compact"}}})
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

    assert_eq!(lines.len(), 6);

    // get_dead_code JSON
    let dead_json: Value = serde_json::from_str(&lines[0]).expect("parse dead json");
    assert_eq!(dead_json["id"], 100);
    assert!(!dead_json["result"]["isError"].as_bool().unwrap_or(false));

    // get_dead_code compact
    let dead_compact: Value = serde_json::from_str(&lines[1]).expect("parse dead compact");
    assert_eq!(dead_compact["id"], 101);
    let dead_compact_text = dead_compact["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(dead_compact_text.starts_with("#SCHEMA: name|kind|file|line"));

    // get_complexity JSON
    let comp_json: Value = serde_json::from_str(&lines[2]).expect("parse comp json");
    assert_eq!(comp_json["id"], 102);
    assert!(!comp_json["result"]["isError"].as_bool().unwrap_or(false));

    // get_complexity compact
    let comp_compact: Value = serde_json::from_str(&lines[3]).expect("parse comp compact");
    assert_eq!(comp_compact["id"], 103);
    let comp_compact_text = comp_compact["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(comp_compact_text.starts_with("#SCHEMA: complexity|name|kind|file|line"));

    // get_hierarchy JSON
    let hier_json: Value = serde_json::from_str(&lines[4]).expect("parse hier json");
    assert_eq!(hier_json["id"], 104);
    assert!(!hier_json["result"]["isError"].as_bool().unwrap_or(false));

    // get_hierarchy compact
    let hier_compact: Value = serde_json::from_str(&lines[5]).expect("parse hier compact");
    assert_eq!(hier_compact["id"], 105);
    let hier_compact_text = hier_compact["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(hier_compact_text.starts_with("#SCHEMA: symbol|kind|file|bases|implementations"));
}

#[test]
fn test_mcp_tool_call_impact_compatibility() {
    let (db, temp) = setup_test_mcp_db();

    let input = format!(
        "{}\n{}\n{}\n{}\n",
        json!({
            "jsonrpc": "2.0",
            "id": 201,
            "method": "tools/call",
            "params": {
                "name": "get_impact",
                "arguments": {
                    "symbol": "helper"
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 202,
            "method": "tools/call",
            "params": {
                "name": "get_impact",
                "arguments": {
                    "symbol_name": "helper"
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 203,
            "method": "tools/call",
            "params": {
                "name": "get_impact",
                "arguments": {
                    "symbol_id": 2
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 204,
            "method": "tools/call",
            "params": {
                "name": "get_impact",
                "arguments": {}
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

    assert_eq!(lines.len(), 4);

    // 1. Calling with "symbol"
    let resp_symbol: Value = serde_json::from_str(&lines[0]).expect("parse symbol");
    assert_eq!(resp_symbol["id"], 201);
    assert!(!resp_symbol["result"]["isError"].as_bool().unwrap_or(false));
    let text_symbol = resp_symbol["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(
        text_symbol.starts_with("**Impact of `helper`"),
        "got: {text_symbol}"
    );

    // 2. Calling with "symbol_name"
    let resp_symbol_name: Value = serde_json::from_str(&lines[1]).expect("parse symbol_name");
    assert_eq!(resp_symbol_name["id"], 202);
    assert!(
        !resp_symbol_name["result"]["isError"]
            .as_bool()
            .unwrap_or(false)
    );
    let text_symbol_name = resp_symbol_name["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert_eq!(text_symbol, text_symbol_name);

    // 3. Calling with "symbol_id"
    let resp_symbol_id: Value = serde_json::from_str(&lines[2]).expect("parse symbol_id");
    assert_eq!(resp_symbol_id["id"], 203);
    assert!(
        !resp_symbol_id["result"]["isError"]
            .as_bool()
            .unwrap_or(false)
    );

    // 4. Missing every param answers with guidance, not an error
    let resp_missing: Value = serde_json::from_str(&lines[3]).expect("parse missing");
    assert_eq!(resp_missing["id"], 204);
    assert!(!resp_missing["result"]["isError"].as_bool().unwrap_or(false));
    let missing_text = resp_missing["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(missing_text.contains("`symbol`"), "got: {missing_text}");
}
#[test]
fn test_mcp_compact_omits_source() {
    let (db, temp) = setup_test_mcp_db();

    let explore_input = format!(
        "{}\n",
        json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/call",
            "params": {
                "name": "graph_explore",
                "arguments": {"query": "execute", "format": "compact"}
            }
        }),
    );
    let mut exp_out = Vec::new();
    run_mcp_server(
        &db,
        Some(temp.path()),
        Cursor::new(explore_input),
        &mut exp_out,
        |_| Ok(json!({"status": "ok"})),
    )
    .expect("run server explore");
    let exp_resp: Value = serde_json::from_str(String::from_utf8(exp_out).expect("utf8").trim())
        .expect("parse explore resp");
    let compact_body = exp_resp["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    // Compact is pipe-delimited metadata — no verbatim source blocks
    assert!(
        compact_body.starts_with("#SCHEMA: id|name|"),
        "compact should start with symbol schema header"
    );
    assert!(
        !compact_body.contains("```"),
        "compact must not contain markdown fenced source blocks"
    );
    assert!(
        !compact_body.contains("pub fn"),
        "compact must not contain verbatim source lines"
    );
}

#[test]
fn a_directory_given_to_the_outline_lists_its_files_and_points_to_explore() {
    let (db, temp) = setup_test_mcp_db();

    let (text, is_error) = call_tool(
        &db,
        temp.path(),
        "get_file_outline",
        json!({"path": "src/"}),
    );

    assert!(!is_error, "got: {text}");
    assert!(text.contains("`src/main.rs`"), "got: {text}");
    assert!(text.contains("graph_explore"), "got: {text}");
}
