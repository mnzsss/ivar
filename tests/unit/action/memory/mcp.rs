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
fn memory_query_finds_an_uncommitted_page_from_a_layout() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    page(
        &layout,
        "docs/memory/gotcha/shadow.md",
        "# Shadow tables\n\nlist them in MEMORY_OBJECTS\n",
    );

    let out = call_json(&layout, "memory_query", json!({"query": "shadow tables"}));

    assert_eq!(out["pages"][0]["path"], "docs/memory/gotcha/shadow.md");
    assert!(out["observations"].as_array().unwrap().is_empty());
}

#[test]
fn memory_query_caps_the_limit_at_twenty() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    for n in 0..25 {
        page(
            &layout,
            &format!("docs/memory/gotcha/g{n:02}.md"),
            "# G\nsharedword\n",
        );
    }
    let out = call_json(
        &layout,
        "memory_query",
        json!({"query": "sharedword", "limit": 500}),
    );
    assert_eq!(out["pages"].as_array().unwrap().len(), 20);
}

#[test]
fn memory_query_rejects_an_unknown_kind() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let error = call(
        &layout,
        "memory_query",
        &json!({"query": "x", "kind": ["nope"]}),
    )
    .unwrap_err();
    assert!(error.contains("nope"), "{error}");
}

#[test]
fn memory_read_page_returns_the_frontmatter_and_body() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    page(
        &layout,
        "docs/memory/decision/pick.md",
        "---\ntitle: Pick one\ntags: [a]\npinned: true\n---\nWhy.\n",
    );

    let out = call_json(
        &layout,
        "memory_read_page",
        json!({"path": "docs/memory/decision/pick.md"}),
    );

    assert_eq!(out["title"], "Pick one");
    assert_eq!(out["tags"], json!(["a"]));
    assert_eq!(out["pinned"], true);
    assert_eq!(out["body"], "Why.\n");
}

#[test]
fn memory_read_page_refuses_paths_outside_the_page_roots() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    for path in [
        "../etc/passwd",
        "/etc/passwd",
        "ivar.json",
        "docs/memory/../../ivar.json",
    ] {
        assert!(
            call(&layout, "memory_read_page", &json!({"path": path})).is_err(),
            "{path}"
        );
    }
}

#[cfg(unix)]
#[test]
fn memory_read_page_refuses_a_symlink_that_leaves_the_hall() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.md"), "# Secret\n").unwrap();
    std::fs::create_dir_all(root.join("docs/memory/gotcha")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.md"),
        root.join("docs/memory/gotcha/link.md"),
    )
    .unwrap();

    let error = call(
        &layout,
        "memory_read_page",
        &json!({"path": "docs/memory/gotcha/link.md"}),
    )
    .unwrap_err();

    assert!(error.contains("symlink"), "{error}");
}

#[cfg(unix)]
#[test]
fn memory_read_page_refuses_symlinks_that_stay_inside_the_hall() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    std::fs::write(root.join(".env"), "SECRET=1\n").unwrap();
    std::fs::create_dir_all(root.join("docs/memory/gotcha")).unwrap();
    std::os::unix::fs::symlink("../../../.env", root.join("docs/memory/gotcha/env.md")).unwrap();
    std::fs::create_dir_all(root.join("docs/other")).unwrap();
    std::fs::write(root.join("docs/other/x.md"), "# X\n").unwrap();
    std::os::unix::fs::symlink("../other", root.join("docs/memory/concept")).unwrap();

    for path in ["docs/memory/gotcha/env.md", "docs/memory/concept/x.md"] {
        let error = call(&layout, "memory_read_page", &json!({"path": path})).unwrap_err();
        assert!(error.contains("symlink"), "{path}: {error}");
    }
}

#[test]
fn memory_schemas_expose_no_scope_arguments_and_only_query_is_always_loaded() {
    for tool in tool_definitions() {
        let props = tool["inputSchema"]["properties"].as_object().unwrap();
        for banned in [
            "workspace",
            "project",
            "cwd",
            "scopes",
            "global",
            "answer",
            "reasoning",
        ] {
            assert!(!props.contains_key(banned), "{} has {banned}", tool["name"]);
        }
        let always = tool["_meta"]["anthropic/alwaysLoad"] == true;
        assert_eq!(always, tool["name"] == "memory_query", "{}", tool["name"]);
    }
}

#[test]
fn memory_review_is_listed_with_its_verdict_schema_and_is_not_read_only() {
    let review = tool_definitions()
        .into_iter()
        .find(|tool| tool["name"] == "memory_review")
        .unwrap();

    assert!(is_memory_tool("memory_review"));
    assert_eq!(
        review["inputSchema"]["properties"]["stale"]["items"]["properties"]["verdict"]["enum"],
        json!(["updated", "still_valid"])
    );
    assert!(review.get("annotations").is_none());
}

#[test]
fn memory_review_outside_a_feature_session_is_an_error() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);

    let error = call(&layout, "memory_review", &json!({})).unwrap_err();

    assert!(error.contains("needs a feature"), "{error}");
    assert!(error.contains("`feature` argument"), "{error}");
}

#[test]
fn memory_review_refuses_a_malformed_verdict() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let caller = MemoryCaller {
        session: Some("s1".to_owned()),
        feature: Some(crate::domain::name::FeatureName::new("login").unwrap()),
        parent_feature: None,
        ..Default::default()
    };

    let error = call_as(
        &layout,
        "memory_review",
        &json!({"confirm": true, "stale": [{"path": "docs/memory/a.md", "verdict": "maybe"}]}),
        &caller,
    )
    .unwrap_err();

    assert!(error.contains("`stale`"), "{error}");
}

#[test]
fn the_ambient_session_owns_a_call_only_when_it_is_a_session_of_this_hall() {
    let (_guard, root) = seeded_hall();
    let (_other_guard, other_root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let ours = crate::domain::name::SessionId::new("6f0c9d5f-0000-4000-8000-00000000000a").unwrap();
    let theirs =
        crate::domain::name::SessionId::new("6f0c9d5f-0000-4000-8000-00000000000b").unwrap();
    crate::test_support::graph_session_view(&root, &ours);
    let their_view = crate::test_support::graph_session_view(&other_root, &theirs);
    let client = McpClient::from_initialize(
        &json!({"clientInfo": {"name": "opencode"}}),
        uuid::Uuid::new_v4(),
    );

    let foreign = caller_in(&layout, &their_view, Some(&client), Some(theirs.as_str()));
    let own = caller_in(&layout, &root, Some(&client), Some(ours.as_str()));

    assert_eq!(foreign.session, None);
    assert_eq!(foreign.owner_key, client.owner_key());
    assert_eq!(own.session.as_deref(), Some(ours.as_str()));
    assert_eq!(own.owner_key, ours.as_str());
}
