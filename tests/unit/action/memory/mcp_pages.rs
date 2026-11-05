#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use serde_json::{Value, json};

use super::*;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

fn page(layout: &Layout, rel: &str, text: &str) {
    let path = layout.root().join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn call_json(layout: &Layout, name: &str, args: Value) -> Value {
    serde_json::from_str(&call(layout, name, &args).unwrap()).unwrap()
}

#[test]
fn an_update_gets_the_next_number_counting_cleanup_records_and_is_queryable_at_once() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    page(
        &layout,
        "docs/memory/update/138-delivery-events.md",
        "# Delivery\n",
    );
    page(
        &layout,
        "docs/memory/update/139-memory-pages.cleanup.json",
        "{}",
    );

    let out = call_json(
        &layout,
        "memory_write_page",
        json!({"kind": "update", "title": "Memory MCP", "body": "Slice 5 landed.\n"}),
    );

    assert_eq!(out["path"], "docs/memory/update/140-memory-mcp.md");
    assert_eq!(out["indexed"], true);
    let found = call_json(&layout, "memory_query", json!({"query": "slice landed"}));
    assert_eq!(
        found["pages"][0]["path"],
        "docs/memory/update/140-memory-mcp.md"
    );
}

#[test]
fn the_frontmatter_carries_only_the_fields_that_were_passed() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    call_json(
        &layout,
        "memory_write_page",
        json!({
            "kind": "decision", "title": "Pick \"one\"", "body": "Why.\n",
            "tags": ["mcp"], "pinned": true,
            "relations": [{"type": "supersedes", "target": "decision/old-pick"}],
            "expires_at": "2027-01-01T00:00:00Z"
        }),
    );

    let source = std::fs::read_to_string(root.join("docs/memory/decision/pick-one.md")).unwrap();
    assert!(
        source.starts_with("---\ntitle: \"Pick \\\"one\\\"\"\n"),
        "{source}"
    );
    assert!(source.contains("tags: [\"mcp\"]"));
    assert!(source.contains("pinned: true"));
    assert!(source.contains("  - type: \"supersedes\"\n    target: \"decision/old-pick\""));
    assert!(!source.contains("kind:"));
    assert!(source.ends_with("---\nWhy.\n"));
}

#[test]
fn an_existing_page_fails_with_its_path_unless_replace_is_set() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    call_json(
        &layout,
        "memory_write_page",
        json!({"kind": "gotcha", "title": "Shadow tables", "body": "v1\n"}),
    );

    let error = call(
        &layout,
        "memory_write_page",
        &json!({"kind": "gotcha", "title": "Shadow tables", "body": "v2\n"}),
    )
    .unwrap_err();
    assert!(
        error.contains("docs/memory/gotcha/shadow-tables.md"),
        "{error}"
    );

    call_json(
        &layout,
        "memory_write_page",
        json!({"kind": "gotcha", "title": "Shadow tables", "body": "v2\n", "replace": true}),
    );
    let read = call_json(
        &layout,
        "memory_read_page",
        json!({"path": "docs/memory/gotcha/shadow-tables.md"}),
    );
    assert_eq!(read["body"], "v2\n");
}

#[test]
fn a_near_equal_title_in_the_same_kind_writes_and_reports_similar() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    call_json(
        &layout,
        "memory_write_page",
        json!({"kind": "gotcha", "title": "FTS shadow tables break doctor"}),
    );

    let out = call_json(
        &layout,
        "memory_write_page",
        json!({"kind": "gotcha", "title": "FTS shadow tables break the doctor"}),
    );

    assert_eq!(
        out["similar"][0]["path"],
        "docs/memory/gotcha/fts-shadow-tables-break-doctor.md"
    );
    assert_eq!(
        out["path"],
        "docs/memory/gotcha/fts-shadow-tables-break-the-doctor.md"
    );
}

#[test]
fn unsafe_or_forbidden_targets_are_refused_and_nothing_is_created() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let cases = [
        json!({"kind": "session", "title": "x"}),
        json!({"kind": "nope", "title": "x"}),
        json!({"kind": "decision", "title": "x", "slug": "../escape"}),
        json!({"kind": "decision", "title": "x", "slug": "Not Kebab"}),
        json!({"kind": "update", "title": "x", "slug": "001-already-numbered"}),
        json!({"kind": "decision"}),
    ];
    for args in cases {
        assert!(call(&layout, "memory_write_page", &args).is_err(), "{args}");
    }
    assert!(!root.join("docs/memory").exists());
}

#[cfg(unix)]
#[test]
fn a_symlinked_kind_directory_is_refused() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.join("docs/memory")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("docs/memory/decision")).unwrap();

    let error = call(
        &layout,
        "memory_write_page",
        &json!({"kind": "decision", "title": "Escape"}),
    )
    .unwrap_err();

    assert!(error.contains("symlink"), "{error}");
    assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
}

#[test]
fn the_write_tool_describes_every_durable_kind_and_forbids_session() {
    let tool = tool_definitions()
        .into_iter()
        .find(|t| t["name"] == "memory_write_page")
        .unwrap();
    let description = tool["description"].as_str().unwrap();
    for kind in [
        "product",
        "update",
        "decision",
        "gotcha",
        "procedure",
        "concept",
    ] {
        assert!(description.contains(kind), "{kind}");
    }
    assert!(description.contains("session"));
    assert_eq!(tool["annotations"]["readOnlyHint"], false);
}

#[cfg(unix)]
#[test]
fn a_page_file_that_is_a_symlink_is_refused_for_write_replace() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("target.md"), "# T\n").unwrap();
    std::fs::create_dir_all(root.join("docs/memory/decision")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("target.md"),
        root.join("docs/memory/decision/linked.md"),
    )
    .unwrap();

    let error = call(
        &layout,
        "memory_write_page",
        &json!({"kind": "decision", "title": "Linked", "replace": true}),
    )
    .unwrap_err();

    assert!(error.contains("symlink"), "{error}");
    assert_eq!(
        std::fs::read_to_string(outside.path().join("target.md")).unwrap(),
        "# T\n"
    );
}

#[test]
fn concurrent_writers_never_share_a_number() {
    let (_guard, root) = seeded_hall();
    let paths: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let layout = Layout::at(root.clone());
                scope.spawn(move || {
                    let out = call_json(
                        &layout,
                        "memory_write_page",
                        json!({"kind": "update", "title": "Same title"}),
                    );
                    out["path"].as_str().unwrap().to_owned()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut unique = paths.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 4, "{paths:?}");
}
