//! Text checks over the shipped `ivar-deliver` skill.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::shipped::shipped;

/// The deliver skill documents PR metadata: global/scoped syntax,
/// inline vs file body, title guidance, and land conflict.
#[test]
fn deliver_skill_documents_pr_metadata() {
    let source = format!(
        "{}/src/harness/skills/ivar-deliver/SKILL.md",
        env!("CARGO_MANIFEST_DIR")
    );
    let body = std::fs::read_to_string(source).unwrap();

    assert!(
        body.contains("--name"),
        "deliver should document --name flag"
    );
    assert!(
        body.contains("--body"),
        "deliver should document --body flag"
    );
    assert!(
        body.contains("--repo api"),
        "deliver should show scoped --repo syntax"
    );
    assert!(
        body.contains("--repo web"),
        "deliver should show multiple scoped repos"
    );
    assert!(
        body.contains("--draft"),
        "deliver should document --draft flag"
    );
    assert!(
        body.contains("./notes.md"),
        "deliver should document file-relative body syntax"
    );
    assert!(
        body.contains("<type>: <short message>"),
        "deliver should guide <type>: <short message> title format"
    );
    assert!(
        body.contains("cannot be used with `--land`"),
        "deliver should state metadata conflicts with land mode"
    );
    assert!(
        !body.contains("L-1234"),
        "deliver should not use a Linear identifier as an example title"
    );
}

#[test]
fn deliver_skill_documents_only_selection() {
    let source = format!(
        "{}/src/harness/skills/ivar-deliver/SKILL.md",
        env!("CARGO_MANIFEST_DIR")
    );
    let body = std::fs::read_to_string(source).unwrap();

    assert!(body.contains("--only"), "deliver should document --only");
    assert!(
        body.contains("merged"),
        "deliver should explain the already-merged PR case"
    );
}

#[test]
fn deliver_links_pull_requests_to_the_t3_thread_when_the_tools_exist() {
    let deliver = shipped("skills/ivar-deliver/SKILL.md");
    assert!(deliver.contains("## Link pull requests to the T3 Code thread"));
    assert!(deliver.contains("`link_pull_request`"));
    assert!(deliver.contains("`list_thread_pull_requests`"));
    assert!(deliver.contains("If those tools are not available"));
}
