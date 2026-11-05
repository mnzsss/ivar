use serde_json::{Value, json};

fn read_only(name: &str, description: &str, properties: &Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "annotations": { "readOnlyHint": true },
        "inputSchema": { "type": "object", "properties": properties, "required": required }
    })
}

pub(super) fn definitions() -> Vec<Value> {
    let mut query = read_only(
        "memory_query",
        "Search the hall's memory pages (decisions, gotchas, procedures, concepts, product and update pages) and, as a fallback in a separate section, captured observations. Call it before proposing architecture or when the user refers to earlier work.",
        &json!({
            "query": { "type": "string", "description": "What to look for" },
            "kind": { "type": "array", "items": { "type": "string" }, "description": "Restrict to these page kinds" },
            "tier": { "type": "string", "enum": ["semantic", "procedural", "episodic", "working"], "description": "Restrict to one tier" },
            "tag": { "type": "string", "description": "Restrict to pages with this tag" },
            "limit": { "type": "integer", "description": "Maximum pages (default 10, maximum 20)" },
            "include_expired": { "type": "boolean", "description": "Also return expired pages" }
        }),
        &["query"],
    );
    if let Some(tool) = query.as_object_mut() {
        tool.insert("_meta".to_owned(), json!({ "anthropic/alwaysLoad": true }));
    }
    vec![
        query,
        read_only(
            "memory_status",
            "Health and counts of the hall memory: observations, spool, retention and indexed pages.",
            &json!({}),
            &[],
        ),
        read_only(
            "memory_read_page",
            "Read one memory page in full: frontmatter fields and body.",
            &json!({
                "path": { "type": "string", "description": "Page path, as memory_query shows it (docs/memory/<kind>/<slug>.md)" }
            }),
            &["path"],
        ),
    ]
}
