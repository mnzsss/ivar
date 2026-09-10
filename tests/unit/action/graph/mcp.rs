#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use serde_json::{Value, json};
use std::io::Cursor;
use tempfile::tempdir;

use super::*;
use crate::domain::graph::{EdgeKind, Provenance, Span, Symbol, SymbolKind};
use crate::store::graph::db::GraphDb;
fn setup_test_mcp_db() -> (GraphDb, tempfile::TempDir) {
    let temp = tempdir().expect("tempdir");
    let hall_root = temp.path();
    let repo_dir = hall_root.join("my-repo");
    std::fs::create_dir_all(repo_dir.join("src")).expect("mkdir src");
    std::fs::create_dir_all(repo_dir.join("tests")).expect("mkdir tests");

    let main_rs = "pub fn execute() {\n    helper();\n}\npub fn helper() {}\n";
    let test_rs = "#[test]\nfn test_exec() {\n    execute();\n}\n";

    std::fs::write(repo_dir.join("src/main.rs"), main_rs).expect("write main.rs");
    std::fs::write(repo_dir.join("tests/exec_test.rs"), test_rs).expect("write test.rs");

    let db = GraphDb::open_in_memory().expect("open db");
    db.insert_repo("my-repo", repo_dir.to_str().unwrap(), "main", None)
        .expect("insert repo");

    let f1 = db
        .upsert_file("my-repo", "src/main.rs", "h1", 100, 100)
        .expect("upsert f1");
    let f2 = db
        .upsert_file("my-repo", "tests/exec_test.rs", "h2", 100, 100)
        .expect("upsert f2");

    let s1 = Symbol {
        id: None,
        file_id: Some(f1),
        repo: "my-repo".to_owned(),
        name: "execute".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("pub fn execute()".to_owned()),
        docstring: Some("Executes main flow.".to_owned()),
        span: Span::new(1, 1, 3, 1),
        is_exported: true,
        complexity: None,
    };
    let s2 = Symbol {
        id: None,
        file_id: Some(f1),
        repo: "my-repo".to_owned(),
        name: "helper".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("pub fn helper()".to_owned()),
        docstring: None,
        span: Span::new(4, 1, 4, 18),
        is_exported: true,
        complexity: None,
    };
    let s3 = Symbol {
        id: None,
        file_id: Some(f2),
        repo: "my-repo".to_owned(),
        name: "test_exec".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("fn test_exec()".to_owned()),
        docstring: None,
        span: Span::new(2, 1, 4, 1),
        is_exported: false,
        complexity: None,
    };

    let ids = db.insert_symbols(&[s1, s2, s3]).expect("insert symbols");
    let s1_id = ids[0];
    let s2_id = ids[1];
    let s3_id = ids[2];

    // execute calls helper
    db.insert_edges(&[
        crate::domain::graph::Edge {
            id: None,
            repo: "my-repo".to_owned(),
            file_id: Some(f1),
            from_symbol_id: Some(s1_id),
            to_symbol_id: Some(s2_id),
            to_name: Some("helper".to_owned()),
            kind: EdgeKind::Calls,
            confidence: 1.0,
            line: 2,
            col: 5,
            provenance: Provenance::Extracted,
        },
        // test_exec calls execute
        crate::domain::graph::Edge {
            id: None,
            repo: "my-repo".to_owned(),
            file_id: Some(f2),
            from_symbol_id: Some(s3_id),
            to_symbol_id: Some(s1_id),
            to_name: Some("execute".to_owned()),
            kind: EdgeKind::Calls,
            confidence: 1.0,
            line: 3,
            col: 5,
            provenance: Provenance::Extracted,
        },
    ])
    .expect("insert edges");

    (db, temp)
}

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
    assert_eq!(
        init_resp["result"]["serverInfo"]["name"],
        "ivar-codebase-graph"
    );
    assert!(
        init_resp["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("graph_explore is Read-equivalent")
    );

    let list_resp: Value = serde_json::from_str(&lines[1]).expect("parse list");
    assert_eq!(list_resp["id"], 2);
    let tools = list_resp["result"]["tools"]
        .as_array()
        .expect("tools array");
    assert_eq!(tools.len(), 11);
    let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
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
fn the_default_tool_surface_lists_only_graph_explore() {
    let tools = super::tools::list_tools(super::tools::ToolSurface::default());
    let names: Vec<&str> = tools
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(names, vec!["graph_explore"]);
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

    let (explore, failed) = call_tool(&db, root, "graph_explore", json!({"path": "src/main.rs"}));
    assert!(!failed, "got: {explore}");
    assert!(explore.contains("`execute`"), "got: {explore}");

    let (missing, missing_failed) = call_tool(&db, root, "graph_explore", json!({}));
    assert!(!missing_failed, "got: {missing}");
    assert!(missing.contains("`query`"), "got: {missing}");
}

fn call_tool(db: &GraphDb, root: &std::path::Path, name: &str, arguments: Value) -> (String, bool) {
    let input = format!(
        "{}\n",
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        })
    );
    let mut output = Vec::new();
    run_mcp_server(db, Some(root), Cursor::new(input), &mut output, |_| {
        Ok(json!({"status": "ok"}))
    })
    .expect("run server");
    let resp: Value =
        serde_json::from_str(String::from_utf8(output).expect("utf8").trim()).expect("parse resp");
    (
        resp["result"]["content"][0]["text"]
            .as_str()
            .expect("text content")
            .to_owned(),
        resp["result"]["isError"].as_bool().unwrap_or(false),
    )
}

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
fn test_mcp_format_guidance_and_compact_no_source() {
    let (db, temp) = setup_test_mcp_db();

    // ── 1. Instructions recommend markdown/omit for discovery ──────────
    let init_input = format!(
        "{}\n",
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
    );
    let mut init_out = Vec::new();
    run_mcp_server(
        &db,
        Some(temp.path()),
        Cursor::new(init_input),
        &mut init_out,
        |_| Ok(json!({"status": "ok"})),
    )
    .expect("run server");
    let init_resp: Value = serde_json::from_str(String::from_utf8(init_out).expect("utf8").trim())
        .expect("parse init");
    let instructions = init_resp["result"]["instructions"]
        .as_str()
        .expect("instructions");
    // The instructions carry what changes behaviour; formats live in the schema
    assert!(
        instructions.contains("do not Read those files again"),
        "instructions should tell agents to treat explore source as read: {instructions}"
    );
    assert!(
        instructions.contains("\"Not shown\""),
        "instructions should point agents from the not-shown list to another explore"
    );

    // ── 2. Tool description carries the same distinction ───────────────
    let list_input = format!(
        "{}\n",
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    );
    let mut list_out = Vec::new();
    run_mcp_server(
        &db,
        Some(temp.path()),
        Cursor::new(list_input),
        &mut list_out,
        |_| Ok(json!({"status": "ok"})),
    )
    .expect("run server");
    let list_resp: Value = serde_json::from_str(String::from_utf8(list_out).expect("utf8").trim())
        .expect("parse list");
    let tools = list_resp["result"]["tools"]
        .as_array()
        .expect("tools array");
    let explore_tool = tools
        .iter()
        .find(|t| t["name"].as_str() == Some("graph_explore"))
        .expect("graph_explore tool present");
    let format_desc = explore_tool["inputSchema"]["properties"]["format"]["description"]
        .as_str()
        .expect("format description");
    assert!(
        format_desc.contains("source snippets"),
        "format param should mention source snippets"
    );
    assert!(
        format_desc.contains("no source"),
        "compact variant should say 'no source'"
    );

    // ── 3. Compact output actually omits source ────────────────────────
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
