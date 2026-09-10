//! Pure synchronous stdio JSON-RPC MCP server for codebase dependency graph queries.
//!
//! Conforms strictly to MCP specification 2024-11-05. Advertises `graph_explore` alone by
//! default, or every graph tool on request. Operates synchronously over standard I/O streams.

pub mod dispatch;
pub mod tools;

use std::io::{self, BufRead, Write};
use std::path::Path;

use serde_json::{Value, json};

use crate::store::graph::db::GraphDb;
pub use dispatch::*;
pub use tools::*;

/// Sent on `initialize`. Hosts that show MCP tools as one-line devices (omp) may
/// never put it in front of the model, so the `graph_explore` description and
/// every answer repeat the parts that change behaviour.
const INSTRUCTIONS: &str = "\
Code graph of this hall: every symbol, call, import and HTTP route, parsed with \
tree-sitter and kept in SQLite.

graph_explore is Read-equivalent. Give it symbol names, file or directory paths, or a \
short intent, several at once, and it returns the verbatim, line-numbered source of the \
relevant files, who depends on them, and the call path between the symbols you name.
- Call it before you Read or grep, and before you edit.
- Treat the source it returns as already Read: do not Read those files again.
- When an answer lists files under \"Not shown\", call graph_explore with those paths or \
names instead of reading them.
- Trust its callers and blast radius; a grep only adds files outside the index.
- A file changed since the last index comes back whole and flagged, so its source stays \
current.";
/// Runs the MCP server loop advertising every graph tool.
pub fn run_mcp_server<R, W, F>(
    db: &GraphDb,
    hall_root: Option<&Path>,
    reader: R,
    writer: W,
    refresh_index: F,
) -> io::Result<()>
where
    R: BufRead,
    W: Write,
    F: FnMut(Option<&str>) -> Result<Value, String>,
{
    run_mcp_server_with_tools(
        db,
        hall_root,
        ToolSurface::All,
        reader,
        writer,
        refresh_index,
    )
}

/// Runs the MCP server loop synchronously reading newline-delimited JSON-RPC from `reader`
/// and writing JSON-RPC responses to `writer`, advertising the tools in `tools`.
pub fn run_mcp_server_with_tools<R, W, F>(
    db: &GraphDb,
    hall_root: Option<&Path>,
    tools: ToolSurface,
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
            if let Some(resp) = handle_json_rpc(db, hall_root, tools, &req, &mut refresh_index) {
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
    tools: ToolSurface,
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
                "tools": list_tools(tools)
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
