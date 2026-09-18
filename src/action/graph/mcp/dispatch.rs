//! Tool call dispatching for Codebase Graph MCP server.

use std::path::Path;

use serde_json::Value;

use super::workspace::WorkspacePaths;
use crate::action::graph::query::QueryError;
use crate::action::graph::query::find::{is_path_like, resolve_query_paths};
use crate::action::graph::{
    affected, compact, complexity, dead_code, explore, hierarchy, narrate, path, query,
};
use crate::store::graph::db::GraphDb;

/// Upper bound on MCP `max_depth`/`max_hops` traversal args. An agent can
/// pass any `u64`; without a ceiling a single call walks the whole graph.
const MAX_DEPTH_LIMIT: usize = 20;
const MAX_HOPS_LIMIT: usize = 20;

/// Reads `key` from `args` as a `u64`, falling back to `default` when absent
/// or not a number, then clamps to `max`.
fn bounded_arg(args: &Value, key: &str, default: usize, max: usize) -> usize {
    args.get(key)
        .and_then(Value::as_u64)
        .map_or(default, |value| usize::try_from(value).unwrap_or(usize::MAX))
        .min(max)
}

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
            let Some(q) = explore_query(args) else {
                return Ok(
                    "`graph_explore` needs `query`: symbol names, an intent such as \"session \
                     enforcement\", or file and directory paths separated by spaces."
                        .to_owned(),
                );
            };
            let repo = args.get("repo").and_then(Value::as_str);

            let root = hall_root
                .ok_or_else(|| "Hall root is required for explore snippet reading".to_owned())?;
            let (requests_files, unindexed) = file_request(db, args, &q, repo)?;
            let mut res = if requests_files {
                explore::explore_files(db, root, &q, repo)
            } else {
                explore::explore(db, root, &q, repo)
            }
            .map_err(|e| format!("explore failed: {e}"))?;
            // A model reads this over MCP to pick its next file, so Markdown is the
            // default; see `narrate`.
            match args.get("format").and_then(Value::as_str) {
                Some("compact") => Ok(compact::encode_explore(&res)),
                Some("json") => serde_json::to_string_pretty(&res).map_err(|e| e.to_string()),
                _ => {
                    WorkspacePaths::from_current_dir().rewrite_explore(db, &mut res);
                    Ok(if requests_files {
                        let mut answer = narrate::narrate_requested_files(&res);
                        if !unindexed.is_empty() {
                            let names: Vec<String> =
                                unindexed.iter().map(|path| format!("`{path}`")).collect();
                            answer.push_str(&format!(
                                "\nNot indexed: {}. No indexed file matches, so Read it directly \
                                 or call `refresh_index` if it is new.\n",
                                names.join(", ")
                            ));
                        }
                        answer
                    } else {
                        narrate::narrate_explore(&res)
                    })
                }
            }
        }

        "get_callers" => {
            let Some(sym) = symbol_arg(args) else {
                return Ok(missing_symbol("get_callers"));
            };
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
            match args.get("format").and_then(Value::as_str) {
                Some("compact") => Ok(compact::encode_callers(&callers)),
                Some("json") => serde_json::to_string_pretty(&callers).map_err(|e| e.to_string()),
                _ => {
                    let mut definitions: Vec<_> = query::find_symbols(db, sym, repo, 5)
                        .map_err(|e| format!("get_callers failed: {e}"))?
                        .into_iter()
                        .filter(|found| found.symbol.name == sym)
                        .collect();
                    let mut references = query::get_references(db, sym, repo)
                        .map_err(|e| format!("get_callers failed: {e}"))?;
                    let mut callers = callers;
                    WorkspacePaths::from_current_dir().rewrite_callers(
                        db,
                        &mut definitions,
                        &mut callers,
                        &mut references,
                    );
                    Ok(narrate::narrate_callers(
                        sym,
                        &definitions,
                        &callers,
                        &references,
                    ))
                }
            }
        }

        "get_callees" => {
            let (symbol_id, symbol_label) =
                if let Some(id) = args.get("symbol_id").and_then(Value::as_i64) {
                    (id, format!("symbol #{id}"))
                } else if let Some(sym) = symbol_arg(args) {
                    let syms = query::find_symbols(db, sym, None, 1)
                        .map_err(|e| format!("failed to find symbol {sym}: {e}"))?;
                    match syms.first().and_then(|s| s.symbol.id) {
                        Some(id) => (id, sym.to_owned()),
                        None => return Ok(unknown_symbol(sym)),
                    }
                } else {
                    return Ok(missing_symbol("get_callees"));
                };

            let callees = query::get_callees(db, symbol_id)
                .map_err(|e| format!("get_callees failed: {e}"))?;
            match args.get("format").and_then(Value::as_str) {
                Some("compact") => Ok(compact::encode_callees(&callees)),
                Some("json") => serde_json::to_string_pretty(&callees).map_err(|e| e.to_string()),
                _ => {
                    let mut callees = callees;
                    WorkspacePaths::from_current_dir().rewrite_callees(db, &mut callees);
                    Ok(narrate::narrate_callees(&symbol_label, &callees))
                }
            }
        }

        "get_file_outline" => {
            let Some(file) = args
                .get("file")
                .or_else(|| args.get("file_path"))
                .or_else(|| args.get("path"))
                .and_then(Value::as_str)
            else {
                return Ok(
                    "`get_file_outline` needs `file`: a file path as shown in the workspace. For \
                     a directory or several files, call `graph_explore` with the paths."
                        .to_owned(),
                );
            };
            let repo = args.get("repo").and_then(Value::as_str);

            let (repo, path) = match (locate_file(db, file, repo)?, repo) {
                (FileTarget::One(repo, path), _) => (repo, path),
                (FileTarget::Several(paths), _) => return Ok(several_files(file, &paths)),
                (FileTarget::Unknown, Some(repo)) => (repo.to_owned(), file.to_owned()),
                (FileTarget::Unknown, None) => return Ok(unknown_file(file)),
            };
            let outline = match query::get_file_outline(db, &repo, &path) {
                Ok(outline) => outline,
                Err(QueryError::FileNotFound { .. }) => return Ok(unknown_file(file)),
                Err(e) => return Err(format!("get_file_outline failed: {e}")),
            };
            if args.get("format").and_then(Value::as_str) == Some("json") {
                serde_json::to_string_pretty(&outline).map_err(|e| e.to_string())
            } else {
                let mut outline = outline;
                WorkspacePaths::from_current_dir().rewrite_outline(db, &mut outline);
                Ok(narrate::narrate_outline(&outline))
            }
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
            let max_depth = bounded_arg(args, "max_depth", 5, MAX_DEPTH_LIMIT);

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
            let max_hops = bounded_arg(args, "max_hops", 6, MAX_HOPS_LIMIT);

            let path_res = path::find_shortest_path(db, from, to, max_hops)
                .map_err(|e| format!("get_path failed: {e}"))?;
            if args.get("format").and_then(Value::as_str) == Some("compact") {
                Ok(compact::encode_path(path_res.as_ref()))
            } else {
                serde_json::to_string_pretty(&path_res).map_err(|e| e.to_string())
            }
        }

        "get_impact" => {
            let max_depth = bounded_arg(args, "max_depth", 5, MAX_DEPTH_LIMIT);
            let symbol_id = if let Some(id) = args.get("symbol_id").and_then(Value::as_i64) {
                id
            } else if let Some(sym_name) = symbol_arg(args) {
                let syms = query::find_symbols(db, sym_name, None, 1)
                    .map_err(|e| format!("failed to find symbol {sym_name}: {e}"))?;
                match syms.first().and_then(|s| s.symbol.id) {
                    Some(id) => id,
                    None => return Ok(unknown_symbol(sym_name)),
                }
            } else {
                return Ok(missing_symbol("get_impact"));
            };

            let impact = query::get_impact(db, symbol_id, max_depth)
                .map_err(|e| format!("get_impact failed: {e}"))?;
            match args.get("format").and_then(Value::as_str) {
                Some("compact") => Ok(compact::encode_impact(&impact)),
                Some("json") => serde_json::to_string_pretty(&impact).map_err(|e| e.to_string()),
                _ => {
                    let mut impact = impact;
                    WorkspacePaths::from_current_dir().rewrite_impact(db, &mut impact);
                    Ok(narrate::narrate_impact(&impact))
                }
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
            let limit = bounded_arg(args, "limit", 50, usize::MAX);
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
            let threshold = u32::try_from(bounded_arg(args, "threshold", 10, u32::MAX as usize))
                .unwrap_or(u32::MAX);
            let limit = bounded_arg(args, "limit", 50, usize::MAX);
            let items = complexity::execute_complexity(db, repo, threshold, limit)
                .map_err(|e| format!("get_complexity failed: {e}"))?;
            if args.get("format").and_then(Value::as_str) == Some("compact") {
                Ok(compact::encode_complexity(&items))
            } else {
                serde_json::to_string_pretty(&items).map_err(|e| e.to_string())
            }
        }

        "get_hierarchy" => {
            let Some(sym) = symbol_arg(args) else {
                return Ok(missing_symbol("get_hierarchy"));
            };
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

enum FileTarget {
    One(String, String),
    Several(Vec<String>),
    Unknown,
}

/// Maps a file argument to the `(repo, path)` pair the index stores. Agents pass
/// the path they see in the workspace, which rarely matches the repo-relative one,
/// and sometimes a directory.
fn locate_file(db: &GraphDb, file: &str, repo: Option<&str>) -> Result<FileTarget, String> {
    let parsed =
        resolve_query_paths(db, file, repo).map_err(|e| format!("get_file_outline failed: {e}"))?;
    if let Some((_, repo, path)) = parsed
        .resolved_paths
        .iter()
        .find(|resolved| resolved.is_pinned())
        .and_then(|resolved| resolved.files().into_iter().next())
    {
        return Ok(FileTarget::One(repo, path));
    }
    let paths: Vec<String> = parsed
        .resolved_paths
        .iter()
        .flat_map(|resolved| resolved.files())
        .map(|(_, _, path)| path)
        .collect();
    Ok(if paths.is_empty() {
        FileTarget::Unknown
    } else {
        FileTarget::Several(paths)
    })
}

fn several_files(file: &str, paths: &[String]) -> String {
    let names: Vec<String> = paths
        .iter()
        .take(20)
        .map(|path| format!("`{path}`"))
        .collect();
    let more = paths.len() - names.len();
    format!(
        "`{file}` matches {} indexed file{}: {}{}. Call `graph_explore` with `{file}` to get \
         their source in one answer, or `get_file_outline` with one of them.",
        paths.len(),
        if paths.len() == 1 { "" } else { "s" },
        names.join(", "),
        if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        },
    )
}

/// Agents send one explore request under any of these keys, several in the same call
/// (an intent with the paths to read), so `explore_query` joins all their values.
const EXPLORE_QUERY_KEYS: [&str; 9] = [
    "query", "queries", "intent", "symbol", "symbols", "path", "paths", "file", "files",
];

fn explore_query(args: &Value) -> Option<String> {
    let terms: Vec<&str> = EXPLORE_QUERY_KEYS
        .iter()
        .filter_map(|key| args.get(*key))
        .flat_map(|value| match value {
            Value::String(term) => vec![term.as_str()],
            Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        })
        .filter(|term| !term.trim().is_empty())
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// Paths mode serves `paths`/`files` arguments, and a query made only of path
/// tokens: agents often list the files they want as free text. Returns whether
/// to serve files whole and the path tokens no indexed file matches.
fn file_request(
    db: &GraphDb,
    args: &Value,
    query: &str,
    repo: Option<&str>,
) -> Result<(bool, Vec<String>), String> {
    let explicit = args.get("paths").is_some() || args.get("files").is_some();
    let tokens: Vec<&str> = query
        .split_whitespace()
        .map(|token| token.trim_matches([',', ';', ':', '"', '\'']))
        .filter(|token| !token.is_empty())
        .collect();
    let all_paths = tokens.iter().all(|token| is_path_like(token));
    if !explicit && !all_paths {
        return Ok((false, Vec::new()));
    }
    let mut unindexed = Vec::new();
    for token in tokens.iter().filter(|token| is_path_like(token)) {
        let parsed =
            resolve_query_paths(db, token, repo).map_err(|e| format!("explore failed: {e}"))?;
        if parsed.resolved_paths.is_empty() {
            unindexed.push((*token).to_owned());
        }
    }
    let serve_files = explicit || unindexed.len() < tokens.len();
    Ok((
        serve_files,
        if serve_files { unindexed } else { Vec::new() },
    ))
}

/// Agents name the symbol argument differently from one call to the next
/// (`symbol`, `name`, `query`), so every symbol tool accepts all of them.
fn symbol_arg(args: &Value) -> Option<&str> {
    ["symbol", "symbol_name", "name", "query"]
        .iter()
        .find_map(|key| args.get(*key).and_then(Value::as_str))
}

fn missing_symbol(tool: &str) -> String {
    format!(
        "`{tool}` needs `symbol`: the name of a function, type or route, such as \
         `evaluateAccess` or `GET /auth/me`."
    )
}

// Misses answer as normal text: an `isError` early in a session teaches the
// agent to stop calling the graph altogether.
fn unknown_symbol(name: &str) -> String {
    format!(
        "No symbol named `{name}` is in the index. Search for it with `graph_explore`, or call \
         `refresh_index` if it was added after the last index."
    )
}

fn unknown_file(file: &str) -> String {
    format!(
        "No indexed file matches `{file}`. Find it with `graph_explore`, or call `refresh_index` \
         if the file is new."
    )
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/mcp/dispatch.rs"]
mod tests;
