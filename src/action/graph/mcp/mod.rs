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

/// Sent on `initialize`, before the agent picks its first tool. It has to say
/// when to use the graph instead of grep: in benchmark2, 5 of 6 runs given a
/// bare capability list ignored an indexed, working graph.
const INSTRUCTIONS: &str = "\
Pre-computed codebase dependency graph (SQLite, cross-repo, sub-millisecond reads). \
It already parsed every symbol, call, and import in this hall, so structural questions \
are a lookup here instead of a re-derivation from file contents.

Reach for this BEFORE grep or reading files whenever the question is structural:
- \"where is X defined / who calls X\" -> graph_explore, get_callers
- \"what breaks if I change X\" -> get_impact, get_affected_tests
- \"how does A reach B\" -> get_path
- \"what is in this file\" -> get_file_outline

graph_explore is the hero call: one query returns matching symbols, their verbatim \
source with line numbers, callers, callees, and blast radius. It replaces the \
grep-then-read-several-files loop, and it follows edges that grep cannot see \
(cross-repo imports, HTTP call sites, dynamic dispatch). Source blocks returned \
are current disk content equivalent to a Read — do not re-read the file after \
graph_explore shows it. Only read excerpts not displayed, or verify low-confidence \
edges (INFERRED / AMBIGUOUS) before relying on them.

Two habits that pay off: query intent (\"session enforcement\"), not just exact \
identifiers, since search is fuzzy; and omit the format parameter (or pass \
format=\"markdown\") when you need source snippets or decision-oriented prose — \
this is the right choice for discovery and for replacing grep+read. Only use \
format=\"compact\" for programmatic parsing of large result sets: it omits source \
snippets and costs fewer tokens, but you lose the context needed to decide what \
to do next.

The index lags edits. After you change code, call refresh_index before trusting \
a structural answer. Results carry provenance and confidence: EXTRACTED is read \
off the AST, INFERRED and AMBIGUOUS are resolved heuristically — verify a \
low-confidence edge by reading the cited line before you rely on it.";
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
