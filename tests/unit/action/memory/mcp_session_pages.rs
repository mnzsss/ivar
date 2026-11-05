#![allow(clippy::unwrap_used)]

use crate::action::memory::mcp;
use crate::store::layout::Layout;
use serde_json::json;

fn hall_with_session_page() -> (tempfile::TempDir, Layout) {
    let (guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let dir = layout.root().join(".ivar/memory/session");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("n1.md"),
        "---\ntitle: \"Conversa\"\n---\n# Conversa\n\ncorpo\n",
    )
    .unwrap();
    (guard, layout)
}

#[test]
fn pinned_and_expires_at_can_be_set_on_a_session_page() {
    let (_g, layout) = hall_with_session_page();
    mcp::call(
        &layout,
        "memory_patch_page",
        &json!({
            "path": ".ivar/memory/session/n1.md", "pinned": true, "expires_at": "2099-01-01T00:00:00Z"
        }),
    )
    .unwrap();
    let text = std::fs::read_to_string(layout.root().join(".ivar/memory/session/n1.md")).unwrap();
    assert!(text.contains("pinned: true"));
    assert!(text.contains("2099-01-01T00:00:00Z"));
    assert!(text.contains("corpo"));
}

#[test]
fn body_tags_and_relations_are_refused_on_a_session_page() {
    let (_g, layout) = hall_with_session_page();
    for args in [
        json!({"path": ".ivar/memory/session/n1.md", "old": "corpo", "new": "x"}),
        json!({"path": ".ivar/memory/session/n1.md", "tags": ["a"]}),
        json!({"path": ".ivar/memory/session/n1.md", "relations": [{"type": "see", "target": "x"}]}),
    ] {
        let error = mcp::call(&layout, "memory_patch_page", &args).unwrap_err();
        assert!(error.contains("only `pinned` and `expires_at`"), "{error}");
    }
}

#[test]
fn other_ephemeral_paths_and_traversal_stay_refused() {
    let (_g, layout) = hall_with_session_page();
    for path in [
        ".ivar/memory/session/../x.md",
        ".ivar/memory/other/n1.md",
        ".ivar/memory/session/sub/n1.md",
    ] {
        assert!(
            mcp::call(
                &layout,
                "memory_patch_page",
                &json!({"path": path, "pinned": true})
            )
            .is_err(),
            "{path}"
        );
    }
}

#[test]
fn write_and_delete_still_refuse_session_pages() {
    let (_g, layout) = hall_with_session_page();
    assert!(
        mcp::call(
            &layout,
            "memory_delete_page",
            &json!({"path": ".ivar/memory/session/n1.md"})
        )
        .is_err()
    );
    assert!(
        mcp::call(
            &layout,
            "memory_write_page",
            &json!({"kind": "session", "title": "x"})
        )
        .is_err()
    );
}
