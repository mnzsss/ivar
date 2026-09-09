//! Tool call dispatching for Codebase Graph MCP server.

use std::path::Path;

use serde_json::Value;

use crate::action::graph::{
    affected, compact, complexity, dead_code, explore, hierarchy, path, query,
};
use crate::store::graph::db::GraphDb;

pub fn dispatch_tool_call<F>(
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
            if args.get("format").and_then(Value::as_str) == Some("compact") {
                Ok(compact::encode_explore(&res))
            } else {
                serde_json::to_string_pretty(&res).map_err(|e| e.to_string())
            }
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
            if args.get("format").and_then(Value::as_str) == Some("compact") {
                Ok(compact::encode_callers(&callers))
            } else {
                serde_json::to_string_pretty(&callers).map_err(|e| e.to_string())
            }
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
            if args.get("format").and_then(Value::as_str) == Some("compact") {
                Ok(compact::encode_callees(&callees))
            } else {
                serde_json::to_string_pretty(&callees).map_err(|e| e.to_string())
            }
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

            let affected =
                affected::find_affected_tests_with_root(db, hall_root, &files, repo, max_depth)
                    .map_err(|e| format!("get_affected_tests failed: {e}"))?;
            if args.get("format").and_then(Value::as_str) == Some("compact") {
                Ok(compact::encode_affected(&affected))
            } else {
                serde_json::to_string_pretty(&affected).map_err(|e| e.to_string())
            }
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
            if args.get("format").and_then(Value::as_str) == Some("compact") {
                Ok(compact::encode_path(path_res.as_ref()))
            } else {
                serde_json::to_string_pretty(&path_res).map_err(|e| e.to_string())
            }
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
            if args.get("format").and_then(Value::as_str) == Some("compact") {
                Ok(compact::encode_impact(&impact))
            } else {
                serde_json::to_string_pretty(&impact).map_err(|e| e.to_string())
            }
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
        "get_dead_code" => {
            let repo = args.get("repo").and_then(Value::as_str);
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
            let items = dead_code::execute_dead_code(db, repo, limit)
                .map_err(|e| format!("get_dead_code failed: {e}"))?;
            if args.get("format").and_then(Value::as_str) == Some("compact") {
                Ok(compact::encode_dead_code(&items))
            } else {
                serde_json::to_string_pretty(&items).map_err(|e| e.to_string())
            }
        }

        "get_complexity" => {
            let repo = args.get("repo").and_then(Value::as_str);
            let threshold = args.get("threshold").and_then(Value::as_u64).unwrap_or(10) as u32;
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
            let items = complexity::execute_complexity(db, repo, threshold, limit)
                .map_err(|e| format!("get_complexity failed: {e}"))?;
            if args.get("format").and_then(Value::as_str) == Some("compact") {
                Ok(compact::encode_complexity(&items))
            } else {
                serde_json::to_string_pretty(&items).map_err(|e| e.to_string())
            }
        }

        "get_hierarchy" => {
            let sym = args
                .get("symbol")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing required parameter 'symbol'".to_owned())?;
            let repo = args.get("repo").and_then(Value::as_str);
            let item = hierarchy::execute_hierarchy(db, sym, repo)
                .map_err(|e| format!("get_hierarchy failed: {e}"))?;
            if args.get("format").and_then(Value::as_str) == Some("compact") {
                Ok(compact::encode_hierarchy(item.as_ref()))
            } else {
                serde_json::to_string_pretty(&item).map_err(|e| e.to_string())
            }
        }

        _ => Err(format!("Unknown tool: {name}")),
    }
}
