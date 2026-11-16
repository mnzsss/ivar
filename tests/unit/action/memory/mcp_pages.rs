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

fn seed_decision(layout: &Layout) {
    call_json(
        layout,
        "memory_write_page",
        json!({"kind": "decision", "title": "Pick one", "body": "Use A because A.\n", "tags": ["old"]}),
    );
}

#[test]
fn patch_replaces_one_exact_occurrence_and_keeps_the_title() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    seed_decision(&layout);

    call_json(
        &layout,
        "memory_patch_page",
        json!({"path": "docs/memory/decision/pick-one.md", "old": "because A", "new": "because B"}),
    );

    let read = call_json(
        &layout,
        "memory_read_page",
        json!({"path": "docs/memory/decision/pick-one.md"}),
    );
    assert_eq!(read["body"], "Use A because B.\n");
    assert_eq!(read["title"], "Pick one");
}

#[test]
fn patch_edits_only_the_body_when_the_same_text_is_in_the_frontmatter() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    page(
        &layout,
        "docs/memory/decision/flag.md",
        "---\npinned: true\n---\ntrue\n",
    );

    call_json(
        &layout,
        "memory_patch_page",
        json!({"path": "docs/memory/decision/flag.md", "old": "true", "new": "false"}),
    );

    let source =
        std::fs::read_to_string(layout.root().join("docs/memory/decision/flag.md")).unwrap();
    assert!(source.starts_with("---\npinned: true\n---\n"), "{source}");
    assert!(source.ends_with("false\n"), "{source}");
}

#[test]
fn patch_refuses_text_that_appears_zero_or_many_times() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    call_json(
        &layout,
        "memory_write_page",
        json!({"kind": "decision", "title": "Twice", "body": "a a\n"}),
    );
    for old in ["missing", "a"] {
        let error = call(
            &layout,
            "memory_patch_page",
            &json!({"path": "docs/memory/decision/twice.md", "old": old, "new": "b"}),
        )
        .unwrap_err();
        assert!(error.contains("exactly once"), "{error}");
    }
}

#[test]
fn patch_changes_metadata_without_touching_kind_title_or_body() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    seed_decision(&layout);

    call_json(
        &layout,
        "memory_patch_page",
        json!({"path": "docs/memory/decision/pick-one.md", "tags": ["new"], "pinned": true}),
    );

    let read = call_json(
        &layout,
        "memory_read_page",
        json!({"path": "docs/memory/decision/pick-one.md"}),
    );
    assert_eq!(read["tags"], json!(["new"]));
    assert_eq!(read["pinned"], true);
    assert_eq!(read["title"], "Pick one");
    assert_eq!(read["kind"], "decision");
    assert_eq!(read["body"], "Use A because A.\n");
}

#[test]
fn patch_and_delete_refuse_session_pages_and_paths_outside_docs_memory() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    for path in [
        ".ivar/memory/session/s.md",
        "docs/memory/session/s.md",
        "../x.md",
        "/etc/passwd",
        "ivar.json",
    ] {
        assert!(
            call(
                &layout,
                "memory_patch_page",
                &json!({"path": path, "old": "a", "new": "b"})
            )
            .is_err(),
            "{path}"
        );
        assert!(
            call(&layout, "memory_delete_page", &json!({"path": path})).is_err(),
            "{path}"
        );
    }
}

#[test]
fn patch_and_delete_refuse_a_page_file_that_is_a_symlink() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    seed_decision(&layout);
    let outside = root.join("outside.md");
    std::fs::write(&outside, "keep\n").unwrap();
    let link = root.join("docs/memory/decision/linked.md");
    std::os::unix::fs::symlink(&outside, &link).unwrap();

    let patch = call(
        &layout,
        "memory_patch_page",
        &json!({"path": "docs/memory/decision/linked.md", "old": "keep", "new": "x"}),
    );
    let delete = call(
        &layout,
        "memory_delete_page",
        &json!({"path": "docs/memory/decision/linked.md"}),
    );

    assert!(patch.unwrap_err().contains("symlink"));
    assert!(delete.unwrap_err().contains("symlink"));
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep\n");
}

#[test]
fn only_the_delete_tool_is_destructive_and_every_read_tool_is_read_only() {
    let tools = tool_definitions();
    for tool in &tools {
        let name = tool["name"].as_str().unwrap();
        let destructive = tool["annotations"]["destructiveHint"] == true;
        assert_eq!(destructive, name == "memory_delete_page", "{name}");
        if name == "memory_review" {
            assert!(tool.get("annotations").is_none(), "{name}");
        } else if matches!(
            name,
            "memory_query"
                | "memory_status"
                | "memory_read_page"
                | "memory_recent"
                | "memory_read_session_observations"
                | "memory_lint"
                | "memory_handoff_list"
                | "memory_message_list"
        ) {
            assert_eq!(tool["annotations"]["readOnlyHint"], true, "{name}");
        } else {
            assert_eq!(tool["annotations"]["readOnlyHint"], false, "{name}");
        }
    }
    assert!(tools.iter().any(|t| t["name"] == "memory_patch_page"));
}

#[test]
fn delete_removes_the_file_and_the_index_row_but_refuses_updates() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    seed_decision(&layout);
    let update = call_json(
        &layout,
        "memory_write_page",
        json!({"kind": "update", "title": "Shipped"}),
    );

    call_json(
        &layout,
        "memory_delete_page",
        json!({"path": "docs/memory/decision/pick-one.md"}),
    );

    assert!(!root.join("docs/memory/decision/pick-one.md").exists());
    let found = call_json(&layout, "memory_query", json!({"query": "pick"}));
    assert!(found["pages"].as_array().unwrap().is_empty());
    let error = call(
        &layout,
        "memory_delete_page",
        &json!({"path": update["path"]}),
    )
    .unwrap_err();
    assert!(error.contains("update"), "{error}");
    assert!(root.join(update["path"].as_str().unwrap()).exists());
}

#[test]
fn a_metadata_patch_keeps_the_declared_kind_and_unknown_fields() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    page(
        &layout,
        "docs/memory/decision/keep.md",
        "---\ntitle: Keep\nkind: decision\nowner: gabriel\nreview:\n  every: monthly\n---\nBody.\n",
    );

    call_json(
        &layout,
        "memory_patch_page",
        json!({"path": "docs/memory/decision/keep.md", "tags": ["new"], "old": "Body", "new": "Text"}),
    );

    let source =
        std::fs::read_to_string(layout.root().join("docs/memory/decision/keep.md")).unwrap();
    for kept in [
        "kind: decision",
        "owner: gabriel",
        "every: monthly",
        "title: Keep",
    ] {
        assert!(source.contains(kept), "{kept} in {source}");
    }
    assert!(source.ends_with("---\nText.\n"), "{source}");
}

#[test]
fn patch_with_new_but_no_old_names_both_arguments() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    seed_decision(&layout);

    let error = call(
        &layout,
        "memory_patch_page",
        &json!({"path": "docs/memory/decision/pick-one.md", "new": "B"}),
    )
    .unwrap_err();

    assert!(
        error.contains("`new`") && error.contains("`old`"),
        "{error}"
    );
}

#[test]
fn patch_clears_expires_at_with_null() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    call_json(
        &layout,
        "memory_write_page",
        json!({"kind": "decision", "title": "Temp", "expires_at": "2099-01-01T00:00:00Z"}),
    );

    call_json(
        &layout,
        "memory_patch_page",
        json!({"path": "docs/memory/decision/temp.md", "expires_at": null}),
    );

    let read = call_json(
        &layout,
        "memory_read_page",
        json!({"path": "docs/memory/decision/temp.md"}),
    );
    assert!(read["expires_at"].is_null(), "{read}");
    let tool = tool_definitions()
        .into_iter()
        .find(|t| t["name"] == "memory_patch_page")
        .unwrap();
    assert_eq!(
        tool["inputSchema"]["properties"]["expires_at"]["type"],
        json!(["string", "null"])
    );
}

#[test]
fn update_pages_cannot_be_patched_or_replaced_without_a_feature() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let update = call_json(
        &layout,
        "memory_write_page",
        json!({"kind": "update", "title": "Shipped", "body": "Done.\n"}),
    );

    let patch = call(
        &layout,
        "memory_patch_page",
        &json!({"path": update["path"], "old": "Done", "new": "Undone"}),
    )
    .unwrap_err();
    let replace = call(
        &layout,
        "memory_write_page",
        &json!({"kind": "update", "title": "Shipped", "replace": true}),
    )
    .unwrap_err();

    assert!(
        patch.contains("update") && patch.contains("delivery record"),
        "{patch}"
    );
    assert!(
        replace.contains("update") && replace.contains("delivery record"),
        "{replace}"
    );
}

#[test]
fn replacing_a_page_does_not_report_it_as_similar_to_itself() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    call_json(
        &layout,
        "memory_write_page",
        json!({"kind": "gotcha", "title": "Shadow tables", "body": "v1\n"}),
    );

    let out = call_json(
        &layout,
        "memory_write_page",
        json!({"kind": "gotcha", "title": "Shadow tables", "body": "v2\n", "replace": true}),
    );

    assert!(out["similar"].as_array().unwrap().is_empty(), "{out}");
}

#[test]
fn read_page_refuses_a_file_over_256_kib() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    page(
        &layout,
        "docs/memory/decision/huge.md",
        &format!("# Huge\n{}", "x".repeat(256 * 1024)),
    );

    let error = call(
        &layout,
        "memory_read_page",
        &json!({"path": "docs/memory/decision/huge.md"}),
    )
    .unwrap_err();

    assert!(error.contains("256 KiB"), "{error}");
}

#[cfg(unix)]
#[test]
fn read_page_refuses_a_fifo_without_blocking() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    std::fs::create_dir_all(root.join("docs/memory/decision")).unwrap();
    let fifo = root.join("docs/memory/decision/pipe.md");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(fifo.as_std_path())
            .status()
            .unwrap()
            .success()
    );

    let error = call(
        &layout,
        "memory_read_page",
        &json!({"path": "docs/memory/decision/pipe.md"}),
    )
    .unwrap_err();

    assert!(error.contains("regular file"), "{error}");
}

mod replace_update {
    use serde_json::{Value, json};

    use super::*;
    use crate::action::memory::mcp::{MemoryCaller, call_as};
    use crate::domain::name::FeatureName;
    use crate::domain::observation::{Importance, Observation};
    use crate::domain::sanitize::{SanitizeContext, Sanitized};
    use crate::store::memory::db::MemoryDb;
    use crate::store::memory::observations::insert_batch;

    fn caller(feature: &str) -> MemoryCaller {
        MemoryCaller {
            session: Some("s1".to_owned()),
            feature: Some(FeatureName::new(feature).unwrap()),
            ..MemoryCaller::default()
        }
    }

    fn write_as(layout: &Layout, who: &MemoryCaller, args: &Value) -> Result<Value, String> {
        call_as(layout, "memory_write_page", args, who)
            .map(|text| serde_json::from_str(&text).unwrap())
    }

    fn record(layout: &Layout, feature: &str, source_event: &str, body: &Value) {
        let observation = Observation {
            event_id: format!("{feature}-{source_event}"),
            source_event: source_event.to_owned(),
            occurred_at: "2026-11-05T10:00:00Z".to_owned(),
            session_id: "s1".to_owned(),
            native_session_id: String::new(),
            provider: String::new(),
            ivar_version: "0".to_owned(),
            feature: feature.to_owned(),
            parent_feature: String::new(),
            repo: String::new(),
            origin: String::new(),
            importance: Importance::memory_page_written(),
            attempt_id: String::new(),
            body: body.to_string(),
        };
        let ctx = SanitizeContext::new(layout.root().to_path_buf(), Vec::new());
        let mut db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
        insert_batch(&mut db, &[Sanitized::sanitize(observation, &ctx).unwrap()]).unwrap();
    }

    fn update_args(body: &str, replace: bool) -> Value {
        json!({"kind": "update", "title": "Shipped", "body": body, "replace": replace})
    }

    fn files(layout: &Layout) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(layout.root().join("docs/memory/update"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_feature_replaces_its_own_undelivered_update_in_place() {
        let (_guard, root) = seeded_hall();
        let layout = Layout::at(root.clone());
        let who = caller("checkout");
        let first = write_as(&layout, &who, &update_args("v1\n", false)).unwrap();
        page(&layout, "docs/memory/update/002-other.md", "# Other\n");

        let second = write_as(&layout, &who, &update_args("v2\n", true)).unwrap();

        assert_eq!(first["path"], "docs/memory/update/001-shipped.md");
        assert_eq!(second["path"], first["path"]);
        assert!(second["similar"].as_array().unwrap().is_empty(), "{second}");
        assert_eq!(files(&layout), ["001-shipped.md", "002-other.md"]);
        let source =
            std::fs::read_to_string(root.join("docs/memory/update/001-shipped.md")).unwrap();
        assert!(source.contains("v2") && !source.contains("v1"), "{source}");
    }

    #[test]
    fn replace_is_refused_once_delivery_began() {
        let (_guard, root) = seeded_hall();
        let layout = Layout::at(root);
        let who = caller("checkout");
        write_as(&layout, &who, &update_args("v1\n", false)).unwrap();
        record(
            &layout,
            "checkout",
            "delivery.checks",
            &json!({"passed": true}),
        );
        write_as(&layout, &who, &update_args("v2\n", true)).unwrap();
        record(&layout, "checkout", "delivery.pushed", &json!({"ok": true}));

        let error = write_as(&layout, &who, &update_args("v3\n", true)).unwrap_err();

        assert!(error.contains("delivery record"), "{error}");
        assert_eq!(files(&layout), ["001-shipped.md"]);
    }

    #[test]
    fn replace_is_refused_after_a_delivered_close() {
        let (_guard, root) = seeded_hall();
        let layout = Layout::at(root);
        let who = caller("checkout");
        write_as(&layout, &who, &update_args("v1\n", false)).unwrap();
        record(
            &layout,
            "checkout",
            "delivery.closed",
            &json!({"outcome": "delivered"}),
        );

        let error = write_as(&layout, &who, &update_args("v2\n", true)).unwrap_err();

        assert!(error.contains("delivery record"), "{error}");
    }

    #[test]
    fn replace_is_refused_for_the_update_of_another_feature() {
        let (_guard, root) = seeded_hall();
        let layout = Layout::at(root);
        write_as(&layout, &caller("checkout"), &update_args("v1\n", false)).unwrap();
        page(
            &layout,
            "docs/memory/update/002-hand-written.md",
            "---\ntitle: \"Hand\"\n---\nx\n",
        );

        let other = write_as(&layout, &caller("billing"), &update_args("v2\n", true)).unwrap_err();
        let unwritten = write_as(
            &layout,
            &caller("checkout"),
            &json!({"kind": "update", "title": "Hand written", "replace": true}),
        )
        .unwrap_err();
        let unbound = write_as(
            &layout,
            &MemoryCaller::default(),
            &update_args("v2\n", true),
        )
        .unwrap_err();

        for error in [other, unwritten, unbound] {
            assert!(
                error.contains("update") && error.contains("delivery record"),
                "{error}"
            );
        }
        assert_eq!(files(&layout), ["001-shipped.md", "002-hand-written.md"]);
    }

    #[test]
    fn patch_and_delete_still_refuse_the_update_of_the_feature() {
        let (_guard, root) = seeded_hall();
        let layout = Layout::at(root);
        let who = caller("checkout");
        let written = write_as(&layout, &who, &update_args("v1\n", false)).unwrap();

        let patch = call_as(
            &layout,
            "memory_patch_page",
            &json!({"path": written["path"], "old": "v1", "new": "v2"}),
            &who,
        )
        .unwrap_err();
        let delete = call_as(
            &layout,
            "memory_delete_page",
            &json!({"path": written["path"]}),
            &who,
        )
        .unwrap_err();

        assert!(patch.contains("delivery record"), "{patch}");
        assert!(delete.contains("delivery record"), "{delete}");
    }

    #[test]
    fn replace_on_a_product_page_overwrites_the_page_with_the_same_slug() {
        let (_guard, root) = seeded_hall();
        let layout = Layout::at(root);
        let args = |body: &str, replace: bool| json!({"kind": "product", "title": "Pricing", "body": body, "replace": replace});
        write_as(&layout, &MemoryCaller::default(), &args("v1\n", false)).unwrap();
        page(&layout, "docs/memory/product/002-other.md", "# Other\n");

        let replaced = write_as(&layout, &MemoryCaller::default(), &args("v2\n", true)).unwrap();
        let fresh = write_as(
            &layout,
            &MemoryCaller::default(),
            &json!({"kind": "product", "title": "Billing", "replace": true}),
        )
        .unwrap();

        assert_eq!(replaced["path"], "docs/memory/product/001-pricing.md");
        assert_eq!(fresh["path"], "docs/memory/product/003-billing.md");
    }
}
