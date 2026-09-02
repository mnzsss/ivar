//! JSON Schema generation for `ivar.json`.
//!
//! Centralizes all schema metadata and customization. The complete
//! draft 2020-12 document is assembled here, with closed objects, the MCP
//! discriminated union (oneOf with exactly `http` and `local` branches),
//! and documented fields matching the runtime types in [`super::model`]
//! and [`crate::domain::mcp`].

use serde_json::{Value, json};

/// Generate the complete JSON Schema describing `ivar.json`.
///
/// Draft 2020-12, with closed objects and the MCP discriminated union
/// (oneOf with exactly `http` and `local` branches).
#[must_use]
#[allow(dead_code)]
pub fn generate() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://ivar.run/ivar.schema.json",
        "title": "ivar.json",
        "description": "The hall configuration file for ivar: identity, providers, repos, integration defaults, skills, and MCP server definitions.",
        "type": "object",
        "required": ["version", "name", "providers", "repos", "integration"],
        "additionalProperties": false,
        "properties": {
            "$schema": {
                "type": "string",
                "description": "URL of the JSON Schema for this file.",
                "const": "https://ivar.run/ivar.schema.json"
            },
            "version": {
                "type": "integer",
                "description": "The manifest schema version.",
                "minimum": 1
            },
            "name": {
                "type": "string",
                "description": "The hall's name: a single, non-empty, non-whitespace, non-traversal path segment."
            },
            "providers": {
                "type": "object",
                "description": "Which harnesses this hall knows about, and which one is the default.",
                "required": ["available", "default"],
                "additionalProperties": false,
                "properties": {
                    "available": {
                        "type": "array",
                        "description": "Every provider id this hall knows about.",
                        "items": {
                            "type": "string",
                            "enum": ["claude-code", "opencode"]
                        },
                        "minItems": 1,
                        "uniqueItems": true
                    },
                    "default": {
                        "type": "string",
                        "description": "The provider ivar picks when none is named explicitly. Must appear in available.",
                        "enum": ["claude-code", "opencode"]
                    }
                }
            },
            "repos": {
                "type": "array",
                "description": "The repos this hall knows about.",
                "items": {
                    "type": "object",
                    "required": ["name", "url", "default_branch"],
                    "additionalProperties": false,
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "A unique, non-empty, single-segment identifier for this repo."
                        },
                        "url": {
                            "type": "string",
                            "description": "The git remote URL to clone from. Must not be empty."
                        },
                        "default_branch": {
                            "type": "string",
                            "description": "The branch a fresh worktree defaults to (e.g. 'main')."
                        },
                        "checks": {
                            "type": "array",
                            "description": "Ordered verification checks, run via 'bash -lc' in the worktree. Empty means no checks.",
                            "items": {
                                "type": "string",
                                "description": "An executable command. Must not be blank."
                            }
                        }
                    }
                }
            },
            "integration": {
                "type": "object",
                "description": "The hall's integration defaults: how features merge and what counts as verified.",
                "required": ["via", "strategy"],
                "additionalProperties": false,
                "properties": {
                    "via": {
                        "type": "string",
                        "description": "How the child's changes travel into the parent.",
                        "enum": ["pr", "local"]
                    },
                    "strategy": {
                        "type": "string",
                        "description": "How the child's commits land on the parent.",
                        "enum": ["squash", "merge", "rebase"]
                    }
                }
            },
            "skills": {
                "type": "object",
                "description": "The hall's shared skill home, if it has one.",
                "required": ["targets"],
                "additionalProperties": false,
                "properties": {
                    "targets": {
                        "type": "object",
                        "description": "Which harnesses this hall's shared skills materialise for.",
                        "required": ["claude", "opencode"],
                        "additionalProperties": false,
                        "properties": {
                            "claude": {
                                "type": "boolean",
                                "description": "Whether skills materialise at .claude/skills/."
                            },
                            "opencode": {
                                "type": "boolean",
                                "description": "Whether skills materialise at .opencode/skills/."
                            }
                        }
                    }
                }
            },
            "mcp": {
                "type": "array",
                "description": "Hall-scoped MCP server definitions materialised by ivar sync.",
                "items": {
                    "oneOf": [mcp_http_branch(), mcp_local_branch()]
                }
            }
        }
    })
}

/// The `http` branch of the MCP oneOf: requires `url`, forbids `command`,
/// `args`, and `env`.
#[allow(dead_code)]
fn mcp_http_branch() -> Value {
    json!({
        "type": "object",
        "description": "A remote MCP server reached over HTTP (SSE or streamable-http).",
        "required": ["name", "type", "url"],
        "additionalProperties": false,
        "properties": {
            "name": {
                "type": "string",
                "description": "The server's name, unique within the hall's manifest."
            },
            "type": {
                "type": "string",
                "const": "http",
                "description": "Transport type: http for remote servers."
            },
            "url": {
                "type": "string",
                "description": "The URL the server is reached at. Must start with http:// or https://."
            },
            "oauth": {
                "type": "object",
                "description": "A pre-provisioned OAuth client registration for servers whose host rejects dynamic client registration.",
                "required": ["client_id", "client_secret_env"],
                "additionalProperties": false,
                "properties": {
                    "client_id": {
                        "type": "string",
                        "description": "The client_id a registration issued. Not a secret."
                    },
                    "client_secret_env": {
                        "type": "string",
                        "description": "The name of the environment variable that holds the client secret at runtime."
                    }
                }
            }
        },
        "not": {
            "required": ["command", "args", "env"]
        }
    })
}

/// The `local` branch of the MCP oneOf: requires `command`, allows `args`,
/// forbids `url`.
fn mcp_local_branch() -> Value {
    json!({
        "type": "object",
        "description": "A local MCP server spawned via stdio.",
        "required": ["name", "type", "command"],
        "additionalProperties": false,
        "properties": {
            "name": {
                "type": "string",
                "description": "The server's name, unique within the hall's manifest."
            },
            "type": {
                "type": "string",
                "const": "local",
                "description": "Transport type: local for servers spawned via stdio."
            },
            "command": {
                "type": "string",
                "description": "The executable to spawn. Must not be blank."
            },
            "args": {
                "type": "array",
                "description": "Arguments appended to the command.",
                "items": {
                    "type": "string"
                }
            },
            "env": {
                "type": "object",
                "description": "Environment variables. Values are references to env var names the harness resolves at runtime, not stored secrets.",
                "additionalProperties": {
                    "type": "string"
                }
            },
            "oauth": {
                "type": "object",
                "description": "A pre-provisioned OAuth client registration for servers whose host rejects dynamic client registration.",
                "required": ["client_id", "client_secret_env"],
                "additionalProperties": false,
                "properties": {
                    "client_id": {
                        "type": "string",
                        "description": "The client_id a registration issued. Not a secret."
                    },
                    "client_secret_env": {
                        "type": "string",
                        "description": "The name of the environment variable that holds the client secret at runtime."
                    }
                }
            }
        },
        "not": {
            "required": ["url"]
        }
    })
}

#[cfg(test)]
#[path = "../../../tests/unit/store/manifest/schema.rs"]
mod tests;
