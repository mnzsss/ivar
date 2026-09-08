//! Pure synchronous stdio JSON-RPC MCP server for codebase dependency graph queries.
//!
//! Conforms strictly to MCP specification 2024-11-05, exposing `graph_explore` hero tool
//! alongside specialized query tools. Operates synchronously over standard I/O streams.

pub mod dispatch;
pub mod tools;

use std::io::{self, BufRead, Write};
use std::path::Path;

use serde_json::{Value, json};

use crate::store::graph::db::GraphDb;
pub use dispatch::*;
pub use tools::*;

const INSTRUCTIONS: &str = "Direct codebase dependency graph. Use graph_explore for fast symbol discovery, verbatim code snippets, call flows, and impact analysis without loading entire files.";
/// Runs the MCP server loop synchronously reading newline-delimited JSON-RPC from `reader`
/// and writing JSON-RPC responses to `writer`.
pub fn run_mcp_server<R, W, F>(
    db: &GraphDb,
    hall_root: Option<&Path>,
    reader: R,
    mut writer: W,
    mut refresh_index: F,
) -> io::Result<()>
where
    R: BufRead,
    W: Write,
    F: FnMut(Option<&str>) -> Result<Value, String>,
{
    for line_res in reader.lines() {
        let line = line_res?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Ok(req) = serde_json::from_str::<Value>(trimmed) {
            if let Some(resp) = handle_json_rpc(db, hall_root, &req, &mut refresh_index) {
                let bytes = serde_json::to_vec(&resp)?;
                writer.write_all(&bytes)?;
                writer.write_all(b"\n")?;
                writer.flush()?;
            }
        } else {
            let err_resp = json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": {
                    "code": -32700,
                    "message": "Parse error: invalid JSON"
                }
            });
            let bytes = serde_json::to_vec(&err_resp)?;
            writer.write_all(&bytes)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }

    Ok(())
}

/// Dispatches a single JSON-RPC request and produces a response value (or None for notifications).
pub fn handle_json_rpc<F>(
    db: &GraphDb,
    hall_root: Option<&Path>,
    req: &Value,
    refresh_index: &mut F,
) -> Option<Value>
where
    F: FnMut(Option<&str>) -> Result<Value, String>,
{
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str)?;

    // If id is null or missing, it's a notification: no response unless error or specified
    if id.is_none() && method.starts_with("notifications/") {
        return None;
    }

    match method {
        "initialize" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "ivar-codebase-graph",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": INSTRUCTIONS
            }
        })),

        "notifications/initialized" | "initialized" => None,

        "ping" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {}
        })),

        "tools/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": list_tools()
            }
        })),

        "tools/call" => {
            let params = req.get("params");
            let tool_name = params.and_then(|p| p.get("name")).and_then(Value::as_str);
            let tool_args = params
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or(json!({}));

            let name = match tool_name {
                Some(n) => n,
                None => {
                    return Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32602,
                            "message": "Missing required parameter 'name'"
                        }
                    }));
                }
            };

            match dispatch_tool_call(db, hall_root, name, &tool_args, refresh_index) {
                Ok(text_content) => Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [
                            {
                                "type": "text",
                                "text": text_content
                            }
                        ]
                    }
                })),
                Err(err_msg) => Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
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
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("Method not found: {method}")
            }
        })),
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/mcp.rs"]
mod tests;
