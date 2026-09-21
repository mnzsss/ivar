//! Pure synchronous stdio JSON-RPC MCP server for codebase dependency graph queries.
//!
//! Conforms strictly to MCP specification 2024-11-05. Advertises `graph_explore` alone by
//! default, or every graph tool on request. Operates synchronously over standard I/O streams.

pub mod dispatch;
pub mod tools;
pub mod workspace;

use std::io::{self, BufRead, Write};
use std::path::Path;

use serde_json::{Value, json};

use crate::action::graph::freshness::ensure_session_freshness;
use crate::action::graph::session::{SessionView, resolve_session_view};
use crate::domain::graph::{UsageEvent, UsageSource};
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
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
    let cwd = camino::Utf8PathBuf::from_path_buf(std::env::current_dir().unwrap_or_default())
        .unwrap_or_default();
    handle_json_rpc_at(db, hall_root, &cwd, tools, req, refresh_index)
}

fn handle_json_rpc_at<F>(
    db: &GraphDb,
    hall_root: Option<&Path>,
    cwd: &camino::Utf8Path,
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

            let started = std::time::Instant::now();
            let refreshed = hall_root.map_or(Ok(None), |root| refresh_hall_session(db, root, cwd));
            let session = refreshed.clone().unwrap_or(None);
            let query = dispatch::explore_query(&tool_args).map(|q| truncate_to_500(&q));
            let outcome = refreshed
                .and_then(|_| dispatch_tool_call(db, hall_root, name, &tool_args, refresh_index));
            let _ = db.record_usage(&UsageEvent {
                command: usage_command(name),
                source: UsageSource::Mcp,
                duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                result_count: outcome.as_ref().ok().and_then(|(_, count)| *count),
                error: outcome.is_err(),
                session,
                query,
            });
            match outcome {
                Ok((text_content, _)) => Some(json!({
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
                Err(err_msg) => Some(tool_error(id.as_ref(), &err_msg)),
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

fn usage_command(name: &str) -> String {
    let known = list_tools(ToolSurface::All)
        .as_array()
        .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == name));
    if known { name } else { "unknown" }.to_owned()
}

const MAX_QUERY_LEN: usize = 500;

pub(super) fn truncate_to_500(s: &str) -> String {
    s.chars().take(MAX_QUERY_LEN).collect()
}

fn refresh_hall_session(
    db: &GraphDb,
    hall_root: &Path,
    cwd: &camino::Utf8Path,
) -> Result<Option<String>, String> {
    let layout =
        Layout::at(camino::Utf8PathBuf::from_path_buf(hall_root.to_path_buf()).unwrap_or_default());
    refresh_session(db, &layout, cwd)
}

fn refresh_session(
    db: &GraphDb,
    layout: &Layout,
    cwd: &camino::Utf8Path,
) -> Result<Option<String>, String> {
    let view = resolve_session_view(layout, cwd)
        .map_err(|err| format!("could not resolve the ivar session for this call: {err}"))?;
    ensure_session_freshness(db, layout, &view).map_err(|err| match &view {
        SessionView::Base { .. } => format!("could not reset the graph to the base view: {err}"),
        SessionView::FeatureSession { feature_name, .. } => format!(
            "the feature layer for `{feature_name}` could not be refreshed, so the graph would answer from stale or base code: {err}"
        ),
    })?;
    Ok(match view {
        SessionView::Base { .. } => None,
        SessionView::FeatureSession { session_id, .. } => session_id,
    })
}

fn tool_error(id: Option<&Value>, err_msg: &str) -> Value {
    json!({
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
    })
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/mcp.rs"]
mod tests;
