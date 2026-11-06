use serde_json::{Value, json};

use super::KIND_DEFINITIONS;

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
        read_only(
            "memory_recent",
            "Newest captured observations first, each with a one-line summary. Pending spool is drained before reading.",
            &json!({
                "limit": { "type": "integer", "description": "Maximum observations (default 20, maximum 100)" },
                "source_event": { "type": "string", "description": "Only observations whose source event starts with this prefix, such as delivery." },
                "feature": { "type": "string", "description": "Only observations of this feature" }
            }),
            &[],
        ),
        read_only(
            "memory_read_session_observations",
            "Read the captured observations of one session, oldest first, a page at a time. Pass next_cursor back as cursor for the next page.",
            &json!({
                "session": { "type": "string", "description": "Session id or native session id" },
                "limit": { "type": "integer", "description": "Maximum observations (default 50, maximum 100)" },
                "cursor": { "type": "integer", "description": "next_cursor of the previous page" }
            }),
            &["session"],
        ),
        write_page(),
        patch_page(),
        delete_page(),
        read_only(
            "memory_lint",
            "Check every page under docs/memory against the page rules on disk: unknown kind, invalid frontmatter, misplaced file, unknown field and markdown links to missing pages. Returns issues with path, code, severity and message.",
            &json!({}),
            &[],
        ),
        review(),
    ]
}

fn review() -> Value {
    json!({
        "name": "memory_review",
        "description": "Review the memory of this session's feature before delivery. Without arguments it lists the pages written in the feature, the gaps (a failed tool without a gotcha, no update page, an analysis approved without a memory_query) and the durable pages the feature's diff may have made stale. With confirm: true it records memory.reviewed, which `ivar feature deliver` requires; every stale page needs a verdict. Gaps never block the confirmation.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "confirm": { "type": "boolean", "description": "Record the review as memory.reviewed" },
                "stale": {
                    "type": "array",
                    "description": "One verdict per possibly stale page",
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "verdict": { "type": "string", "enum": ["updated", "still_valid"] }
                        },
                        "required": ["path", "verdict"]
                    }
                },
                "not_required": { "type": "string", "description": "Why the feature needs no page; refused when it wrote one" }
            },
            "required": []
        }
    })
}

fn write_page() -> Value {
    json!({
        "name": "memory_write_page",
        "description": format!("Write a page under docs/memory/<kind>/. Never commits. Kinds: {KIND_DEFINITIONS}. The kind `session` is refused: only the consolidation writes it."),
        "annotations": { "readOnlyHint": false },
        "inputSchema": {
            "type": "object",
            "properties": {
                "kind": { "type": "string", "description": "Page kind" },
                "title": { "type": "string", "description": "Page title; the slug derives from it" },
                "body": { "type": "string", "description": "Markdown body" },
                "slug": { "type": "string", "description": "Kebab-case slug; defaults to the title. Product and update pages get their NNN- number from the tool, so omit it" },
                "tags": { "type": "array", "items": { "type": "string" } },
                "pinned": { "type": "boolean" },
                "relations": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": { "type": { "type": "string" }, "target": { "type": "string" } },
                        "required": ["type", "target"]
                    }
                },
                "expires_at": { "type": "string", "description": "RFC 3339 expiry" },
                "replace": { "type": "boolean", "description": "Overwrite an existing page of the same path (default false)" }
            },
            "required": ["kind", "title"]
        }
    })
}

fn patch_page() -> Value {
    json!({
        "name": "memory_patch_page",
        "description": "Edit one durable or session page in place. Replace one exact piece of the body with old/new, and/or change tags, pinned, relations or expires_at. Kind and title cannot change. `new` needs `old`. Refuses `update` pages. On a .ivar/memory/session/<id>.md page only pinned and expires_at may change. Never commits.",
        "annotations": { "readOnlyHint": false },
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Page path (docs/memory/<kind>/<slug>.md or .ivar/memory/session/<id>.md)" },
                "old": { "type": "string", "description": "Body text to replace; must appear exactly once" },
                "new": { "type": "string", "description": "Replacement text (default empty)" },
                "tags": { "type": "array", "items": { "type": "string" } },
                "pinned": { "type": "boolean" },
                "relations": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": { "type": { "type": "string" }, "target": { "type": "string" } },
                        "required": ["type", "target"]
                    }
                },
                "expires_at": { "type": ["string", "null"], "description": "RFC 3339 expiry; null removes it" }
            },
            "required": ["path"]
        }
    })
}

fn delete_page() -> Value {
    json!({
        "name": "memory_delete_page",
        "description": "Delete one durable page file. Never commits. Refuses `update` pages, which belong to the delivery record.",
        "annotations": { "readOnlyHint": false, "destructiveHint": true },
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Page path (docs/memory/<kind>/<slug>.md)" }
            },
            "required": ["path"]
        }
    })
}
