//! JSON Schema generation for `ivar.json`.
//!
//! Uses `schemars` to derive the schema from the Rust types, then
//! post-processes to add metadata, override the MCP discriminated union,
//! and constrain `version` to [`CURRENT_VERSION`]. All metadata and
//! customization is centralized here; field-level docs ride as doc comments
//! on the types (schemars turns them into `description`).

use serde_json::{Value, json};

use super::model::{CURRENT_VERSION, MANIFEST_SCHEMA_URL};

/// Generate the complete JSON Schema describing `ivar.json`.
///
/// Draft 2020-12, with closed objects and the MCP discriminated union
/// (oneOf with exactly `http` and `local` branches). The base schema is
/// derived from the Rust types via `schemars`; only the MCP entry, metadata,
/// version constraint, and provider-array constraints are hand-applied.
#[must_use]
#[allow(clippy::expect_used, clippy::indexing_slicing)]
pub fn generate() -> Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(super::model::Manifest))
        .expect("schema must serialize");

    // ── metadata ──────────────────────────────────────────────────────
    schema["$id"] = json!(MANIFEST_SCHEMA_URL);
    schema["title"] = json!("ivar.json");
    schema["description"] = json!(
        "The hall configuration file for ivar: identity, providers, repos, \
         integration defaults, skills, and MCP server definitions."
    );

    // ── version: const CURRENT_VERSION (not minimum: 1) ──────────────
    schema["properties"]["version"] = json!({
        "type": "integer",
        "description": "The manifest schema version.",
        "const": CURRENT_VERSION
    });

    // ── $schema: const URL ────────────────────────────────────────────
    schema["properties"]["$schema"] = json!({
        "type": "string",
        "description": "URL of the JSON Schema for this file.",
        "const": MANIFEST_SCHEMA_URL
    });

    // ── providers.available: minItems + uniqueItems ────────────────────
    if let Some(available) = schema.pointer_mut("/$defs/Providers/properties/available") {
        available["minItems"] = json!(1);
        available["uniqueItems"] = json!(true);
    }

    // ── MCP: override with approved discriminated union ───────────────
    let oauth_schema = extract_oauth_schema(&schema);
    schema["properties"]["mcp"] = json!({
        "type": "array",
        "description": "Hall-scoped MCP server definitions materialised by ivar sync.",
        "items": {
            "oneOf": [
                mcp_http_branch(oauth_schema.clone()),
                mcp_local_branch(oauth_schema)
            ]
        }
    });

    // ── Remove the orphan McpServerDef from $defs ────────────────────
    // The MCP override replaces the mcp property with hand-built oneOf
    // branches. The schemars-derived McpServerDef is the loose runtime
    // shape (all fields optional/nullable) which contradicts the strict
    // oneOf and is never referenced via $ref. Remove it so the artifact
    // carries no dead, contradictory definition.
    if let Some(defs) = schema.pointer_mut("/$defs")
        && let Some(obj) = defs.as_object_mut()
    {
        obj.remove("McpServerDef");
    }

    schema
}

// ── MCP oneOf helpers ───────────────────────────────────────────────────────

/// Extract the `McpOauth` schema from the generated `$defs` so the oneOf
/// branches reference the type-derived definition rather than a hand-written
/// duplicate.
fn extract_oauth_schema(schema: &Value) -> Value {
    if let Some(defs) = schema.get("$defs").and_then(Value::as_object) {
        for (key, val) in defs {
            if key == "McpOauth" || key.ends_with("::McpOauth") || key.ends_with("__McpOauth") {
                return val.clone();
            }
        }
    }
    // Fallback — should never be reached when the derive is in place.
    json!({
        "type": "object",
        "description": "A pre-provisioned OAuth client registration for \
            servers whose host rejects dynamic client registration.",
        "required": ["client_id"],
        "additionalProperties": false,
        "properties": {
            "client_id": {
                "type": "string",
                "description": "The client_id a registration issued. Not a secret."
            },
            "client_secret_env": {
                "type": "string",
                "description": "The name of the environment variable that holds \
                    the client secret at runtime."
            }
        }
    })
}

/// The `http` branch of the MCP oneOf: requires `name`, `type` (const
/// `"http"`), and `url`; forbids `command`, `args`, and `env` via
/// `additionalProperties: false` (no `not` block — see deviation 2 fix).
fn mcp_http_branch(oauth_schema: Value) -> Value {
    json!({
        "type": "object",
        "description": "A remote MCP server reached over HTTP.",
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
            "oauth": oauth_schema
        }
    })
}

/// The `local` branch of the MCP oneOf: requires `name`, `type` (const
/// `"local"`), and `command`; allows optional `args`, `env`, `oauth`;
/// forbids `url` via `additionalProperties: false` (no `not` block).
fn mcp_local_branch(oauth_schema: Value) -> Value {
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
            "oauth": oauth_schema
        }
    })
}

#[cfg(test)]
#[path = "../../../tests/unit/store/manifest/schema.rs"]
mod tests;
