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
    assert!(tools.contains("The default surface lists 12 tools"));
    assert!(tools.contains("all twelve tools"));
    assert!(doc("docs/reference/limitations-memory.md").contains("## The memory review"));
    assert!(doc("docs/reference/limitations-delivery.md").contains("--skip-memory-review"));
    assert!(doc("docs/guides/day-to-day.md").contains("`deliver.memory_not_reviewed`"));
    assert!(
        !doc("docs/guides/day-to-day.md")
            .contains("approvals for delivery, documentation, and teardown")
    );
    assert!(doc("README.md").contains("eight more memory tools"));
}
