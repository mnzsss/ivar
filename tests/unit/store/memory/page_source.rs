#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::*;
use crate::domain::memory_page::{PageIssueCode, PageKind};

fn codes(path: &str, source: &str) -> Vec<PageIssueCode> {
    validate_page(path, source)
        .into_iter()
        .map(|i| i.code)
        .collect()
}

#[test]
fn a_page_without_frontmatter_is_valid_and_titled_by_its_h1() {
    let page = parse_page(
        "docs/memory/product/003-worktree-teardown.md",
        "# Worktree teardown\n\nbody\n",
    )
    .unwrap();
    assert_eq!(page.kind, PageKind::Product);
    assert_eq!(page.title, "Worktree teardown");
    assert!(page.meta.tags.is_empty() && !page.meta.pinned);
}

#[test]
fn the_title_falls_back_to_the_slug() {
    let page = parse_page("docs/memory/gotcha/fts-shadow-tables.md", "no heading\n").unwrap();
    assert_eq!(page.title, "fts-shadow-tables");
}

#[test]
fn frontmatter_fields_are_read() {
    let source = "---\ntitle: Index\ntags: [fts, index]\npinned: true\nexpires_at: 2027-01-01T00:00:00Z\nrelations:\n  - type: supersedes\n    target: decision/old-index\n---\n# Ignored\n";
    let page = parse_page("docs/memory/decision/index.md", source).unwrap();
    assert_eq!(page.title, "Index");
    assert_eq!(page.meta.tags, ["fts", "index"]);
    assert!(page.meta.pinned);
    assert_eq!(page.meta.relations[0].target, "decision/old-index");
}

#[test]
fn invalid_pages_are_named_by_code() {
    assert_eq!(
        codes("docs/memory/rule/x.md", "# x\n"),
        [PageIssueCode::KindUnknown]
    );
    assert_eq!(
        codes("docs/memory/gotcha/x.md", "---\nkind: decision\n---\n# x\n"),
        [PageIssueCode::KindMismatch]
    );
    assert_eq!(
        codes("docs/memory/gotcha/sub/x.md", "# x\n"),
        [PageIssueCode::Misplaced]
    );
    assert_eq!(
        codes("docs/memory/gotcha/Bad_Slug.md", "# x\n"),
        [PageIssueCode::Misplaced]
    );
    assert_eq!(
        codes("docs/memory/update/no-number.md", "# x\n"),
        [PageIssueCode::Misplaced]
    );
    assert_eq!(
        codes(
            "docs/memory/gotcha/x.md",
            "---\nexpires_at: tomorrow\n---\n# x\n"
        ),
        [PageIssueCode::ExpiresAtInvalid]
    );
    assert_eq!(
        codes("docs/memory/gotcha/x.md", "---\ntags: 3\n---\n# x\n"),
        [PageIssueCode::FrontmatterInvalid]
    );
    assert_eq!(
        codes("docs/memory/gotcha/x.md", "---\nunterminated\n# x\n"),
        [PageIssueCode::FrontmatterInvalid]
    );
}

#[test]
fn an_unknown_key_is_a_warning_the_index_ignores() {
    let source = "---\ntier: semantic\n---\n# x\n";
    let issues = validate_page("docs/memory/gotcha/x.md", source);
    assert_eq!(issues.len(), 1);
    assert!(!issues[0].is_error());
    let page = parse_page("docs/memory/gotcha/x.md", source).unwrap();
    assert_eq!(page.unknown_fields, ["tier"]);
}

#[test]
fn an_ephemeral_root_accepts_session_pages_only() {
    assert!(codes(".ivar/memory/session/abc.md", "# s\n").is_empty());
    assert_eq!(
        codes("docs/memory/session/abc.md", "# s\n"),
        [PageIssueCode::KindUnknown]
    );
}
