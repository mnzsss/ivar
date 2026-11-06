//! The documents the memory MCP slice writes stay under the commit hook's token limit.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[test]
fn the_memory_mcp_documents_fit_the_commit_hook_limit() {
    for rel in [
        "docs/reference/mcp-tools.md",
        "docs/reference/limitations-memory.md",
        "docs/adr/0011-one-mcp-server-for-graph-and-memory.md",
        "docs/architecture/module-map.md",
        "docs/architecture/seams.md",
        "README.md",
    ] {
        let path = format!("{}/{rel}", env!("CARGO_MANIFEST_DIR"));
        let bytes = std::fs::metadata(&path)
            .unwrap_or_else(|_| panic!("{rel} missing"))
            .len();
        assert!(bytes < 20_000, "{rel} is {bytes} bytes");
    }
}

fn doc(rel: &str) -> String {
    std::fs::read_to_string(format!("{}/{rel}", env!("CARGO_MANIFEST_DIR")))
        .unwrap_or_else(|_| panic!("{rel} missing"))
}

#[test]
fn the_docs_describe_the_memory_review_gate() {
    let tools = doc("docs/reference/mcp-tools.md");
    assert!(tools.contains("| `memory_review` |"));
    assert!(tools.contains("The default surface lists 20 tools"));
    assert!(tools.contains("all twenty tools"));
    assert!(doc("docs/reference/limitations-memory.md").contains("## The memory review"));
    assert!(doc("docs/reference/limitations-delivery.md").contains("--skip-memory-review"));
    assert!(doc("docs/guides/day-to-day.md").contains("`deliver.memory_not_reviewed`"));
    assert!(
        !doc("docs/guides/day-to-day.md")
            .contains("approvals for delivery, documentation, and teardown")
    );
    assert!(doc("README.md").contains("sixteen more memory tools"));
}

#[test]
fn the_docs_describe_handoffs_and_messages() {
    let tools = doc("docs/reference/mcp-tools.md");
    for tool in [
        "memory_handoff_begin",
        "memory_handoff_list",
        "memory_handoff_accept",
        "memory_handoff_cancel",
        "memory_message_send",
        "memory_message_list",
        "memory_message_pop",
        "memory_message_cancel",
    ] {
        assert!(tools.contains(&format!("| `{tool}` |")), "{tool} missing from mcp-tools.md");
    }
    assert!(tools.contains("### Handoffs and messages"));

    let limits = doc("docs/reference/limitations-memory.md");
    assert!(limits.contains("## Handoffs and messages"));
    assert!(limits.contains("OpenCode has no brief"));
    assert!(limits.contains("`<provider>:mcp-<uuid>`"));

    let databases = doc("docs/reference/on-disk-format-databases.md");
    assert!(databases.contains("### Coordination (schema 5)"), "task 01 owns this section");

    let versions = doc("docs/reference/on-disk-format-versions.md");
    assert!(versions.contains("handoff_ttl_days"));

    let readme = doc("README.md");
    assert!(readme.contains("ivar memory handoff"));
    assert!(readme.contains("ivar memory brief"));
}
