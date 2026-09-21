//! Tool schema definitions for Codebase Graph MCP server.

use serde_json::{Value, json};

/// Which tools `tools/list` advertises.
///
/// Hosts such as omp list every MCP tool as a line of the system prompt. With
/// only `graph_explore` listed, an agent chains explore calls instead of mixing
/// in callers, greps and reads; CodeGraph ships the same default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum ToolSurface {
    /// Only `graph_explore`.
    #[default]
    Explore,
    /// Every graph tool.
    All,
}

pub fn list_tools(surface: ToolSurface) -> Value {
    let tools = all_tools();
    match surface {
        ToolSurface::All => tools,
        ToolSurface::Explore => Value::Array(
            tools
                .as_array()
                .into_iter()
                .flatten()
                .filter(|tool| tool["name"] == "graph_explore")
                .cloned()
                .collect(),
        ),
    }
}

fn tool_graph_explore() -> Value {
    json!({
        "name": "graph_explore",
        "description": "PRIMARY TOOL, call it first for any question about this code and before any edit: it returns the verbatim, line-numbered source of the relevant files (treat it as already Read), who depends on them, and the call path between the symbols you name. Query with symbol names, file or directory paths, or a short intent, several at once. When an answer lists files under \"Not shown\", send the `Next:` call it gives. Files requested via `paths` come back as full source, cheaper than reading them one by one.",
        "_meta": { "anthropic/alwaysLoad": true },
        "annotations": { "readOnlyHint": true },
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Symbol names, file or directory paths, or a short intent, several at once (e.g. \"login apiRequest\" or \"services/api/src/routes/auth.ts services/api/src/routes/admin.ts\")" },
                "paths": { "type": "array", "items": { "type": "string" }, "description": "Files to return whole, as the paths an answer names" },
                "repo": { "type": "string", "description": "Optional repository filter" },
                "format": { "type": "string", "enum": ["markdown", "json", "compact"], "description": "Output format: 'markdown' (default, includes source snippets — best for discovery and replacing grep+read), 'json' for raw struct, or 'compact' (pipe-delimited, no source — for programmatic parsing of large results)" }
            }
        }
    })
}

fn tool_get_callers() -> Value {
    json!({
        "name": "get_callers",
        "description": "Every call site and type use of a symbol across repos, plus where it is defined and which files import it. Use instead of grepping for the symbol name: this covers every indexed file, resolves imports and aliases, and skips matches in comments and unrelated identifiers.",
        "annotations": { "readOnlyHint": true },
        "inputSchema": {
            "type": "object",
            "properties": {
                "symbol": { "type": "string", "description": "Symbol name" },
                "repo": { "type": "string", "description": "Optional repository filter" },
                "cross_repo": { "type": "boolean", "description": "Whether to search cross-repository edges" },
                "min_confidence": { "type": "number", "description": "Minimum edge confidence score (0.0 - 1.0)" },
                "format": { "type": "string", "enum": ["markdown", "json", "compact"], "description": "Output format: 'markdown' (default, one line per call site), 'json' for raw struct, or 'compact' (pipe-delimited)" }
            }
        }
    })
}

fn tool_get_callees() -> Value {
    json!({
        "name": "get_callees",
        "description": "Every outgoing call a symbol makes. Use to learn what a function depends on without reading its body and chasing each import by hand.",
        "annotations": { "readOnlyHint": true },
        "inputSchema": {
            "type": "object",
            "properties": {
                "symbol_id": { "type": "integer", "description": "Symbol row ID" },
                "symbol": { "type": "string", "description": "Symbol name (if symbol_id is unknown)" },
                "format": { "type": "string", "enum": ["markdown", "json", "compact"], "description": "Output format: 'markdown' (default, one line per call), 'json' for raw struct, or 'compact' (pipe-delimited)" }
            }
        }
    })
}

fn tool_get_file_outline() -> Value {
    json!({
        "name": "get_file_outline",
        "description": "All symbols and imports declared in one file, with line numbers. Cheaper than reading the file when you only need its shape before deciding what to open.",
        "annotations": { "readOnlyHint": true },
        "inputSchema": {
            "type": "object",
            "properties": {
                "file": { "type": "string", "description": "File path, as shown in the workspace or relative to its repository" },
                "repo": { "type": "string", "description": "Optional repository name, when the path alone is ambiguous" },
                "format": { "type": "string", "enum": ["markdown", "json"], "description": "Output format: 'markdown' (default) or 'json' for raw struct" }
            }
        }
    })
}

fn tool_get_affected_tests() -> Value {
    json!({
        "name": "get_affected_tests",
        "description": "Test files transitively reachable from the source files you changed. Use to pick which tests to run instead of running the whole suite or guessing by filename.",
        "annotations": { "readOnlyHint": true },
        "inputSchema": {
            "type": "object",
            "properties": {
                "files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Modified file paths"
                },
                "repo": { "type": "string", "description": "Optional repository filter" },
                "max_depth": { "type": "integer", "description": "Maximum traversal depth (default 5)" },
                "format": { "type": "string", "enum": ["json", "compact"], "description": "Output format: 'json' (default) or 'compact' (token-efficient pipe-delimited)" }
            },
            "required": ["files"]
        }
    })
}

fn tool_get_path() -> Value {
    json!({
        "name": "get_path",
        "description": "Shortest call/import chain connecting two symbols, hop by hop. Use to answer 'how does A reach B' across layers or repos, which grep cannot reconstruct.",
        "annotations": { "readOnlyHint": true },
        "inputSchema": {
            "type": "object",
            "properties": {
                "from": { "type": "string", "description": "Starting symbol name" },
                "to": { "type": "string", "description": "Target symbol name" },
                "max_hops": { "type": "integer", "description": "Maximum path hops (default 6)" },
                "format": { "type": "string", "enum": ["json", "compact"], "description": "Output format: 'json' (default) or 'compact' (token-efficient pipe-delimited)" }
            },
            "required": ["from", "to"]
        }
    })
}

fn tool_get_impact() -> Value {
    json!({
        "name": "get_impact",
        "description": "Everything transitively affected by changing a symbol: which consumers, how deep, in which files. Run this BEFORE editing a shared symbol to find the callers you would otherwise miss.",
        "annotations": { "readOnlyHint": true },
        "inputSchema": {
            "type": "object",
            "properties": {
                "symbol_id": { "type": "integer", "description": "Symbol row ID" },
                "symbol_name": { "type": "string", "description": "Symbol name (if ID unknown)" },
                "symbol": { "type": "string", "description": "Alias for symbol_name" },
                "max_depth": { "type": "integer", "description": "Maximum depth (default 5)" },
                "format": { "type": "string", "enum": ["markdown", "json", "compact"], "description": "Output format: 'markdown' (default, grouped by depth), 'json' for raw struct, or 'compact' (pipe-delimited)" }
            }
        }
    })
}

fn tool_refresh_index() -> Value {
    json!({
        "name": "refresh_index",
        "description": "Re-index after you edit code. The graph is a snapshot: call this before trusting a structural answer that must reflect your latest changes.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "repo": { "type": "string", "description": "Optional repository name. Omit for all repos." }
            }
        }
    })
}

fn tool_get_dead_code() -> Value {
    json!({
        "name": "get_dead_code",
        "description": "Find unreferenced private symbols and dead code in the codebase.",
        "annotations": { "readOnlyHint": true },
        "inputSchema": {
            "type": "object",
            "properties": {
                "repo": { "type": "string", "description": "Optional repository filter" },
                "limit": { "type": "integer", "description": "Maximum number of items to return (default 50)" },
                "format": { "type": "string", "enum": ["json", "compact"], "description": "Output format: 'json' (default) or 'compact' (token-efficient pipe-delimited)" }
            }
        }
    })
}

fn tool_get_complexity() -> Value {
    json!({
        "name": "get_complexity",
        "description": "Find functions and methods ranked descending by cyclomatic complexity.",
        "annotations": { "readOnlyHint": true },
        "inputSchema": {
            "type": "object",
            "properties": {
                "threshold": { "type": "integer", "description": "Minimum cyclomatic complexity threshold (default 10)" },
                "repo": { "type": "string", "description": "Optional repository filter" },
                "limit": { "type": "integer", "description": "Maximum number of items to return (default 50)" },
                "format": { "type": "string", "enum": ["json", "compact"], "description": "Output format: 'json' (default) or 'compact' (token-efficient pipe-delimited)" }
            }
        }
    })
}

fn tool_get_hierarchy() -> Value {
    json!({
        "name": "get_hierarchy",
        "description": "Analyze class, struct, and trait inheritance/implementation hierarchy for a symbol.",
        "annotations": { "readOnlyHint": true },
        "inputSchema": {
            "type": "object",
            "properties": {
                "symbol": { "type": "string", "description": "Symbol name to query hierarchy for" },
                "repo": { "type": "string", "description": "Optional repository filter" },
                "format": { "type": "string", "enum": ["json", "compact"], "description": "Output format: 'json' (default) or 'compact' (token-efficient pipe-delimited)" }
            }
        }
    })
}

fn all_tools() -> Value {
    Value::Array(vec![
        tool_graph_explore(),
        tool_get_callers(),
        tool_get_callees(),
        tool_get_file_outline(),
        tool_get_affected_tests(),
        tool_get_path(),
        tool_get_impact(),
        tool_refresh_index(),
        tool_get_dead_code(),
        tool_get_complexity(),
        tool_get_hierarchy(),
        tool_graph_feedback(),
    ])
}

fn tool_graph_feedback() -> Value {
    json!({
        "name": "graph_feedback",
        "description": "Call this when a graph_explore or other graph tool answer was empty, wrong, or unhelpful for the question you actually had. Records the query you asked and why it fell short, so the miss can be reviewed later. Always succeeds; it never blocks or corrects your current answer.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "The query or symbol you asked the graph about" },
                "reason": { "type": "string", "description": "Why the answer was empty or unhelpful" }
            },
            "required": ["query", "reason"]
        }
    })
}
