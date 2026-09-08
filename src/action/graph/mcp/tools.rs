//! Tool schema definitions for Codebase Graph MCP server.

use serde_json::{Value, json};

pub fn list_tools() -> Value {
    json!([
        {
            "name": "graph_explore",
            "description": "Multi-hop context explorer that returns exact code snippets, callers, and callees for an intent/query.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query or symbol name" },
                    "repo": { "type": "string", "description": "Optional repository filter" },
                    "format": { "type": "string", "enum": ["json", "compact"], "description": "Output format: 'json' (default) or 'compact' (token-efficient pipe-delimited)" }
                },
                "required": ["query"]
            }
        },
        {
            "name": "get_callers",
            "description": "Find all incoming callers of a symbol across repositories.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Symbol name" },
                    "repo": { "type": "string", "description": "Optional repository filter" },
                    "cross_repo": { "type": "boolean", "description": "Whether to search cross-repository edges" },
                    "min_confidence": { "type": "number", "description": "Minimum edge confidence score (0.0 - 1.0)" },
                    "format": { "type": "string", "enum": ["json", "compact"], "description": "Output format: 'json' (default) or 'compact' (token-efficient pipe-delimited)" }
                },
                "required": ["symbol"]
            }
        },
        {
            "name": "get_callees",
            "description": "Find all outgoing targets/callees called by a symbol.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol_id": { "type": "integer", "description": "Symbol row ID" },
                    "symbol": { "type": "string", "description": "Symbol name (if symbol_id is unknown)" },
                    "format": { "type": "string", "enum": ["json", "compact"], "description": "Output format: 'json' (default) or 'compact' (token-efficient pipe-delimited)" }
                }
            }
        },
        {
            "name": "get_file_outline",
            "description": "Get structural outline of symbols and imports in a file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "File path" },
                    "repo": { "type": "string", "description": "Repository ID" }
                },
                "required": ["file", "repo"]
            }
        },
        {
            "name": "get_affected_tests",
            "description": "Find test files transitively affected by changes to given source files.",
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
        },
        {
            "name": "get_path",
            "description": "Find shortest path between two symbols in the call graph.",
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
        },
        {
            "name": "get_impact",
            "description": "Transitive blast-radius impact analysis of changing a symbol.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol_id": { "type": "integer", "description": "Symbol row ID" },
                    "symbol_name": { "type": "string", "description": "Symbol name (if ID unknown)" },
                    "max_depth": { "type": "integer", "description": "Maximum depth (default 5)" },
                    "format": { "type": "string", "enum": ["json", "compact"], "description": "Output format: 'json' (default) or 'compact' (token-efficient pipe-delimited)" }
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
        },
        {
            "name": "get_dead_code",
            "description": "Find unreferenced private symbols and dead code in the codebase.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repo": { "type": "string", "description": "Optional repository filter" },
                    "limit": { "type": "integer", "description": "Maximum number of items to return (default 50)" },
                    "format": { "type": "string", "enum": ["json", "compact"], "description": "Output format: 'json' (default) or 'compact' (token-efficient pipe-delimited)" }
                }
            }
        },
        {
            "name": "get_complexity",
            "description": "Find functions and methods ranked descending by cyclomatic complexity.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "threshold": { "type": "integer", "description": "Minimum cyclomatic complexity threshold (default 10)" },
                    "repo": { "type": "string", "description": "Optional repository filter" },
                    "limit": { "type": "integer", "description": "Maximum number of items to return (default 50)" },
                    "format": { "type": "string", "enum": ["json", "compact"], "description": "Output format: 'json' (default) or 'compact' (token-efficient pipe-delimited)" }
                }
            }
        },
        {
            "name": "get_hierarchy",
            "description": "Analyze class, struct, and trait inheritance/implementation hierarchy for a symbol.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Symbol name to query hierarchy for" },
                    "repo": { "type": "string", "description": "Optional repository filter" },
                    "format": { "type": "string", "enum": ["json", "compact"], "description": "Output format: 'json' (default) or 'compact' (token-efficient pipe-delimited)" }
                },
                "required": ["symbol"]
            }
        }
    ])
}
