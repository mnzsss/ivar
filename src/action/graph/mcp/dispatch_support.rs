use std::fmt::Write as _;

use serde_json::Value;

use crate::action::graph::query::find::{is_path_like, resolve_query_paths, split_line_range};
use crate::domain::graph::{
    ExploreConfidence, ExploreResult, MissEvent, MissKind, truncate_for_storage,
};
use crate::store::graph::db::GraphDb;

/// Records an explore answer made only of weak candidates, so `ivar graph
/// misses --kind weak` shows which intent queries the index cannot answer.
/// Best-effort: the answer is already built and must not depend on this write.
pub(super) fn record_weak_miss(
    db: &GraphDb,
    session: Option<&str>,
    query: &str,
    res: &ExploreResult,
) {
    let named_candidates = !res.primary_symbols.is_empty() || !res.file_matches.is_empty();
    if res.confidence != ExploreConfidence::NoMatch || !named_candidates {
        return;
    }
    let _ = db.record_miss(&MissEvent {
        session: session.map(str::to_owned),
        kind: MissKind::Weak,
        query: Some(truncate_for_storage(query)),
        pattern: None,
        reason: Some("weak".to_owned()),
    });
}

pub(super) enum FileTarget {
    One(String, String),
    Several(Vec<String>),
    Unknown,
}

/// Maps a file argument to the `(repo, path)` pair the index stores. Agents pass
/// the path they see in the workspace, which rarely matches the repo-relative one,
/// and sometimes a directory.
pub(super) fn locate_file(
    db: &GraphDb,
    file: &str,
    repo: Option<&str>,
) -> Result<FileTarget, String> {
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

pub(super) fn several_files(file: &str, paths: &[String]) -> String {
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
pub(super) const EXPLORE_QUERY_KEYS: [&str; 9] = [
    "query", "queries", "intent", "symbol", "symbols", "path", "paths", "file", "files",
];

/// Paths mode serves `paths`/`files` arguments, and a query made only of path
/// tokens: agents often list the files they want as free text. Returns whether
/// to serve files whole and the path tokens no indexed file matches.
pub(super) fn file_request(
    db: &GraphDb,
    args: &Value,
    query: &str,
    repo: Option<&str>,
) -> Result<(bool, Vec<String>), String> {
    let explicit = args.get("paths").is_some() || args.get("files").is_some();
    let tokens: Vec<&str> = query
        .split_whitespace()
        .map(|token| token.trim_matches([',', ';', '"', '\'']))
        .filter(|token| !token.is_empty())
        .collect();
    let all_paths = tokens.iter().all(|token| {
        let (path, _) = split_line_range(token);
        is_path_like(path)
    });
    if !explicit && !all_paths {
        return Ok((false, Vec::new()));
    }
    let mut unindexed = Vec::new();
    for token in &tokens {
        let (path, _) = split_line_range(token);
        if is_path_like(path) {
            let parsed =
                resolve_query_paths(db, path, repo).map_err(|e| format!("explore failed: {e}"))?;
            if parsed.resolved_paths.is_empty() {
                unindexed.push((*token).to_owned());
            }
        }
    }
    let serve_files = explicit || unindexed.len() < tokens.len();
    Ok((
        serve_files,
        if serve_files { unindexed } else { Vec::new() },
    ))
}

/// Prefixes the answer with the tree each explicit worktree path was read from.
pub(super) fn label_layers(labels: &[String], text: String) -> String {
    if labels.is_empty() {
        return text;
    }
    let mut out = String::new();
    for label in labels {
        let _ = writeln!(out, "Source: {label}");
    }
    out.push('\n');
    out.push_str(&text);
    out
}

/// Agents name the symbol argument differently from one call to the next
/// (`symbol`, `name`, `query`), so every symbol tool accepts all of them.
pub(super) fn symbol_arg(args: &Value) -> Option<&str> {
    ["symbol", "symbol_name", "name", "query"]
        .iter()
        .find_map(|key| args.get(*key).and_then(Value::as_str))
}

pub(super) fn missing_symbol(tool: &str) -> String {
    format!(
        "`{tool}` needs `symbol`: the name of a function, type or route, such as \
         `evaluateAccess` or `GET /auth/me`."
    )
}

// Misses answer as normal text: an `isError` early in a session teaches the
// agent to stop calling the graph altogether.
pub(super) fn unknown_symbol(name: &str) -> String {
    format!(
        "No symbol named `{name}` is in the index. Search for it with `graph_explore`, or call \
         `refresh_index` if it was added after the last index."
    )
}

pub(super) fn unknown_file(file: &str) -> String {
    format!(
        "No indexed file matches `{file}`. Find it with `graph_explore`, or call `refresh_index` \
         if the file is new."
    )
}
