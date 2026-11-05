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
