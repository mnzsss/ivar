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
            .contains("codebase dependency graph")
    );

    let list_resp: Value = serde_json::from_str(&lines[1]).expect("parse list");
    assert_eq!(list_resp["id"], 2);
    let tools = list_resp["result"]["tools"]
        .as_array()
        .expect("tools array");
    assert_eq!(tools.len(), 12);
    let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(tool_names.contains(&"graph_explore"));
    assert!(tool_names.contains(&"get_callers"));
    assert!(tool_names.contains(&"get_callees"));
    assert!(tool_names.contains(&"get_file_outline"));
    assert!(tool_names.contains(&"get_affected_tests"));
    assert!(tool_names.contains(&"get_path"));
    assert!(tool_names.contains(&"get_impact"));
    assert!(tool_names.contains(&"refresh_index"));
    assert!(tool_names.contains(&"get_graph_stats"));
    assert!(tool_names.contains(&"get_dead_code"));
    assert!(tool_names.contains(&"get_complexity"));
    assert!(tool_names.contains(&"get_hierarchy"));
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
    let content = &resp["result"]["content"][0]["text"];
    assert!(content.as_str().unwrap().contains("execute"));
    assert!(content.as_str().unwrap().contains("helper"));
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
