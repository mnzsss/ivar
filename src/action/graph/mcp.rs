//! Pure synchronous stdio JSON-RPC MCP server for codebase dependency graph queries.
//!
//! Conforms strictly to MCP specification 2024-11-05, exposing `graph_explore` hero tool
//! alongside specialized query tools. Operates synchronously over standard I/O streams.

use std::io::{self, BufRead, Write};
use std::path::Path;

use serde_json::{Value, json};

use crate::action::graph::affected;
use crate::action::graph::explore;
use crate::action::graph::path;
use crate::action::graph::query;
use crate::store::graph::db::GraphDb;

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "ivar-codebase-graph";
const SERVER_VERSION: &str = "0.9.1";
const INSTRUCTIONS: &str = "Direct codebase dependency graph. Use graph_explore for fast symbol discovery, verbatim code snippets, call flows, and impact analysis without loading entire files.";

/// Runs the synchronous MCP server over given reader and writer streams.
pub fn run_mcp_server<R: BufRead, W: Write, F>(
    db: &GraphDb,
    hall_root: Option<&Path>,
    mut reader: R,
    mut writer: W,
    mut refresh_index: F,
) -> io::Result<()>
where
    F: FnMut(Option<&str>) -> Result<Value, String>,
{
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break; // EOF
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parsed: Result<Value, serde_json::Error> = serde_json::from_str(trimmed);
        let req = match parsed {
            Ok(v) => v,
            Err(err) => {
                let err_resp = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": {
                        "code": -32700,
                        "message": format!("Parse error: {err}")
                    }
                });
                write_json_line(&mut writer, &err_resp)?;
                continue;
            }
        };

        if let Some(resp) = handle_json_rpc(db, hall_root, &req, &mut refresh_index) {
            write_json_line(&mut writer, &resp)?;
        }
    }

    Ok(())
}

fn write_json_line<W: Write>(writer: &mut W, value: &Value) -> io::Result<()> {
    let mut out = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_owned());
    out.push('\n');
    writer.write_all(out.as_bytes())?;
    writer.flush()
}

fn handle_json_rpc<F>(
    db: &GraphDb,
    hall_root: Option<&Path>,
    req: &Value,
    refresh_index: &mut F,
) -> Option<Value>
where
    F: FnMut(Option<&str>) -> Result<Value, String>,
{
    let id = req.get("id").cloned();
    let method = match req.get("method").and_then(Value::as_str) {
        Some(m) => m,
        None => {
            // Notification or malformed
            return id.map(|id_val| {
                json!({
                    "jsonrpc": "2.0",
                    "id": id_val,
                    "error": {
                        "code": -32600,
                        "message": "Invalid Request: missing method"
                    }
                })
            });
        }
    };

    // If it is a notification (no id), we don't respond except when needed
    id.as_ref()?;
    let id_val = id.unwrap_or(Value::Null);

    let params = req.get("params").unwrap_or(&Value::Null);

    match method {
        "initialize" => Some(json!({
            "jsonrpc": "2.0",
            "id": id_val,
            "result": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": SERVER_NAME,
                    "version": SERVER_VERSION
                },
                "instructions": INSTRUCTIONS
            }
        })),

        "ping" => Some(json!({
            "jsonrpc": "2.0",
            "id": id_val,
            "result": {}
        })),

        "tools/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id_val,
            "result": {
                "tools": list_tools()
            }
        })),

        "tools/call" => {
            let tool_name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let arguments = params.get("arguments").unwrap_or(&Value::Null);

            match dispatch_tool_call(db, hall_root, tool_name, arguments, refresh_index) {
                Ok(content_text) => Some(json!({
                    "jsonrpc": "2.0",
                    "id": id_val,
                    "result": {
                        "content": [
                            {
                                "type": "text",
                                "text": content_text
                            }
                        ]
                    }
                })),
                Err(err_msg) => Some(json!({
                    "jsonrpc": "2.0",
                    "id": id_val,
                    "result": {
                        "content": [
                            {
                                "type": "text",
                                "text": format!("Error: {err_msg}")
                            }
                        ],
                        "isError": true
                    }
                })),
            }
        }

        _ => Some(json!({
            "jsonrpc": "2.0",
            "id": id_val,
            "error": {
                "code": -32601,
                "message": format!("Method not found: {method}")
            }
        })),
    }
}

fn list_tools() -> Value {
    json!([
        {
            "name": "graph_explore",
            "description": "Hero query synthesizing symbol discovery, surgical source snippet, callers, callees, and impact analysis.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Symbol name or search query" },
                    "repo": { "type": "string", "description": "Optional repository filter" }
                },
                "required": ["query"]
            }
        },
        {
            "name": "get_callers",
            "description": "Find direct callers of a symbol.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Symbol name" },
                    "repo": { "type": "string", "description": "Optional repository filter" },
                    "limit": { "type": "integer", "description": "Max results" }
                },
                "required": ["symbol"]
            }
        },
        {
            "name": "get_callees",
            "description": "Find direct callees from a symbol.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Symbol name" },
                    "repo": { "type": "string", "description": "Optional repository filter" },
                    "limit": { "type": "integer", "description": "Max results" }
                },
                "required": ["symbol"]
            }
        },
        {
            "name": "get_file_outline",
            "description": "Show file outline with symbols and imports.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "File path" },
                    "repo": { "type": "string", "description": "Optional repository filter" }
                },
                "required": ["file"]
            }
        },
        {
            "name": "get_affected_tests",
            "description": "Find reverse-dependent test files for changed files.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "files": { "type": "array", "items": { "type": "string" }, "description": "List of changed files" },
                    "repo": { "type": "string", "description": "Optional repository filter" },
                    "max_depth": { "type": "integer", "description": "Max traversal depth (default 5)" }
                },
                "required": ["files"]
            }
        },
        {
            "name": "get_path",
            "description": "Find shortest path between two symbols or files.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Source symbol name or file path" },
                    "to": { "type": "string", "description": "Target symbol name or file path" },
                    "max_hops": { "type": "integer", "description": "Max traversal hops (default 6)" }
                },
                "required": ["from", "to"]
            }
        },
        {
            "name": "get_impact",
            "description": "Transitive blast-radius impact analysis for a symbol.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol_id": { "type": "integer", "description": "Symbol ID" },
                    "symbol_name": { "type": "string", "description": "Or symbol name" },
                    "max_depth": { "type": "integer", "description": "Max depth (default 5)" }
                }
            }
        },
        {
            "name": "refresh_index",
            "description": "Incrementally index a repository or all repositories.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repo": { "type": "string", "description": "Optional repository name. Omit for all repos." }
                }
            }
        },
        {
            "name": "get_graph_stats",
            "description": "Get overall graph statistics: repositories, files, symbols, edges count.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }
    ])
}

fn dispatch_tool_call<F>(
    db: &GraphDb,
    hall_root: Option<&Path>,
    name: &str,
    args: &Value,
    refresh_index: &mut F,
) -> Result<String, String>
where
    F: FnMut(Option<&str>) -> Result<Value, String>,
{
    match name {
        "graph_explore" => {
            let q = args
                .get("query")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing required parameter 'query'".to_owned())?;
            let repo = args.get("repo").and_then(Value::as_str);

            let root = hall_root
                .ok_or_else(|| "Hall root is required for explore snippet reading".to_owned())?;
            let res =
                explore::explore(db, root, q, repo).map_err(|e| format!("explore failed: {e}"))?;
            serde_json::to_string_pretty(&res).map_err(|e| e.to_string())
        }

        "get_callers" => {
            let sym = args
                .get("symbol")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing required parameter 'symbol'".to_owned())?;
            let repo = args.get("repo").and_then(Value::as_str);
            let cross_repo = args
                .get("cross_repo")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let min_confidence = args
                .get("min_confidence")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);

            let callers = query::get_callers(db, sym, repo, cross_repo, min_confidence)
                .map_err(|e| format!("get_callers failed: {e}"))?;
            serde_json::to_string_pretty(&callers).map_err(|e| e.to_string())
        }

        "get_callees" => {
            let symbol_id = if let Some(id) = args.get("symbol_id").and_then(Value::as_i64) {
                id
            } else if let Some(sym) = args.get("symbol").and_then(Value::as_str) {
                let syms = query::find_symbols(db, sym, None, 1)
                    .map_err(|e| format!("failed to find symbol {sym}: {e}"))?;
                if let Some(first) = syms.first() {
                    first.symbol.id.unwrap_or(0)
                } else {
                    return Err(format!("Symbol '{sym}' not found"));
                }
            } else {
                return Err("Either 'symbol_id' or 'symbol' must be provided".to_owned());
            };

            let callees = query::get_callees(db, symbol_id)
                .map_err(|e| format!("get_callees failed: {e}"))?;
            serde_json::to_string_pretty(&callees).map_err(|e| e.to_string())
        }

        "get_file_outline" => {
            let file = args
                .get("file")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing required parameter 'file'".to_owned())?;
            let repo = args
                .get("repo")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing required parameter 'repo'".to_owned())?;

            let outline = query::get_file_outline(db, repo, file)
                .map_err(|e| format!("get_file_outline failed: {e}"))?;
            serde_json::to_string_pretty(&outline).map_err(|e| e.to_string())
        }

        "get_affected_tests" => {
            let files_arr = args
                .get("files")
                .and_then(Value::as_array)
                .ok_or_else(|| "Missing required parameter 'files'".to_owned())?;
            let files: Vec<String> = files_arr
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            let repo = args.get("repo").and_then(Value::as_str);
            let max_depth = args.get("max_depth").and_then(Value::as_u64).unwrap_or(5) as usize;

            let affected = affected::find_affected_tests(db, &files, repo, max_depth)
                .map_err(|e| format!("get_affected_tests failed: {e}"))?;
            serde_json::to_string_pretty(&affected).map_err(|e| e.to_string())
        }

        "get_path" => {
            let from = args
                .get("from")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing required parameter 'from'".to_owned())?;
            let to = args
                .get("to")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing required parameter 'to'".to_owned())?;
            let max_hops = args.get("max_hops").and_then(Value::as_u64).unwrap_or(6) as usize;

            let path_res = path::find_shortest_path(db, from, to, max_hops)
                .map_err(|e| format!("get_path failed: {e}"))?;
            serde_json::to_string_pretty(&path_res).map_err(|e| e.to_string())
        }

        "get_impact" => {
            let max_depth = args.get("max_depth").and_then(Value::as_u64).unwrap_or(5) as usize;
            let symbol_id = if let Some(id) = args.get("symbol_id").and_then(Value::as_i64) {
                id
            } else if let Some(sym_name) = args.get("symbol_name").and_then(Value::as_str) {
                let syms = query::find_symbols(db, sym_name, None, 1)
                    .map_err(|e| format!("failed to find symbol {sym_name}: {e}"))?;
                if let Some(first) = syms.first() {
                    first.symbol.id.unwrap_or(0)
                } else {
                    return Err(format!("Symbol '{sym_name}' not found"));
                }
            } else {
                return Err("Either 'symbol_id' or 'symbol_name' must be provided".to_owned());
            };

            let impact = query::get_impact(db, symbol_id, max_depth)
                .map_err(|e| format!("get_impact failed: {e}"))?;
            serde_json::to_string_pretty(&impact).map_err(|e| e.to_string())
        }

        "refresh_index" => {
            let target_repo = args.get("repo").and_then(Value::as_str);
            let res = refresh_index(target_repo)?;
            serde_json::to_string_pretty(&res).map_err(|e| e.to_string())
        }

        "get_graph_stats" => {
            let stats =
                query::get_graph_stats(db).map_err(|e| format!("get_graph_stats failed: {e}"))?;
            serde_json::to_string_pretty(&stats).map_err(|e| e.to_string())
        }

        _ => Err(format!("Unknown tool: {name}")),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/mcp.rs"]
mod tests;
