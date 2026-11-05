mod confine;
mod instructions;
mod observations;
mod tools;

use serde_json::{Value, json};

use crate::action::memory::query::{QueryInput, query_in};
use crate::action::memory::status::status_in;
use crate::store::layout::Layout;
use crate::store::memory::page_source::parse_page;

pub use instructions::{KIND_DEFINITIONS, MEMORY_INSTRUCTIONS};

const MAX_LIMIT: usize = 20;
const DEFAULT_LIMIT: usize = 10;

pub const TOOL_NAMES: &[&str] = &[
    "memory_query",
    "memory_status",
    "memory_read_page",
    "memory_recent",
    "memory_read_session_observations",
];

#[must_use]
pub fn is_memory_tool(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

#[must_use]
pub fn tool_definitions() -> Vec<Value> {
    tools::definitions()
}

/// # Errors
///
/// The message the MCP answer carries as `isError`.
pub fn call(layout: &Layout, name: &str, args: &Value) -> Result<String, String> {
    match name {
        "memory_query" => query(layout, args),
        "memory_status" => to_text(&status_in(layout).map_err(|failure| failure.what)?.value),
        "memory_read_page" => read_page(layout, args),
        "memory_recent" => observations::recent(layout, args),
        "memory_read_session_observations" => observations::session(layout, args),
        other => Err(format!("Unknown tool: {other}")),
    }
}

fn to_text<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}

fn string_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn query(layout: &Layout, args: &Value) -> Result<String, String> {
    let text = string_arg(args, "query").ok_or("`memory_query` needs `query`")?;
    let kinds = args
        .get("kind")
        .and_then(Value::as_array)
        .map(|kinds| {
            kinds
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_LIMIT, |n| {
            usize::try_from(n).map_or(MAX_LIMIT, |n| n.min(MAX_LIMIT))
        });
    let input = QueryInput {
        text,
        kinds,
        tier: string_arg(args, "tier"),
        tag: string_arg(args, "tag"),
        limit,
        include_expired: args
            .get("include_expired")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        explain: false,
    };
    to_text(
        &query_in(layout, &input)
            .map_err(|failure| failure.what)?
            .value,
    )
}

fn read_page(layout: &Layout, args: &Value) -> Result<String, String> {
    let rel = string_arg(args, "path").ok_or("`memory_read_page` needs `path`")?;
    let path = confine::resolve_readable(layout, &rel)?;
    let source = std::fs::read_to_string(&path)
        .map_err(|error| format!("could not read `{rel}`: {error}"))?;
    let page = parse_page(&rel, &source).map_err(|issues| {
        let messages: Vec<&str> = issues.iter().map(|issue| issue.message.as_str()).collect();
        format!("`{rel}` is not a valid page: {}", messages.join("; "))
    })?;
    to_text(&json!({
        "path": rel,
        "kind": page.kind.as_str(),
        "title": page.title,
        "tags": page.meta.tags,
        "pinned": page.meta.pinned,
        "relations": page
            .meta
            .relations
            .iter()
            .map(|relation| json!({"type": relation.kind, "target": relation.target}))
            .collect::<Vec<_>>(),
        "expires_at": page.meta.expires_at,
        "body": page.body,
    }))
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/memory/mcp.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../../tests/unit/action/memory/mcp_observations.rs"]
mod observation_tests;
