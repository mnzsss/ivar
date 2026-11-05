#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::*;
use crate::domain::memory_page::PageKind;

fn targets(body: &str) -> Vec<LinkTarget> {
    extract_links(body).into_iter().map(|l| l.target).collect()
}

#[test]
fn wikilinks_name_a_slug_or_a_kind_and_slug() {
    assert_eq!(
        targets("see [[fts-shadow-tables]] and [[decision/old-index]]"),
        [
            LinkTarget::Slug("fts-shadow-tables".into()),
            LinkTarget::KindSlug(PageKind::Decision, "old-index".into()),
        ]
    );
}

#[test]
fn markdown_links_to_pages_use_the_last_two_path_segments() {
    assert_eq!(
        targets("[a](../decision/old-index.md#why) [b](https://example.com/x.md) [c](notes.txt)"),
        [LinkTarget::KindSlug(PageKind::Decision, "old-index".into())]
    );
}

#[test]
fn code_is_not_scanned() {
    let body = "`[[inline]]`\n\n```\n[[fenced]]\n```\n\n[[real]]\n";
    assert_eq!(targets(body), [LinkTarget::Slug("real".into())]);
}

#[test]
fn relation_targets_are_kind_and_slug() {
    assert_eq!(
        relation_target("decision/old-index"),
        Some(LinkTarget::KindSlug(PageKind::Decision, "old-index".into()))
    );
    assert_eq!(relation_target("no-kind"), None);
}

#[test]
fn relations_become_typed_links() {
    let relations = [
        Relation {
            kind: "supersedes".into(),
            target: "decision/old-index".into(),
        },
        Relation {
            kind: "see".into(),
            target: "no-kind".into(),
        },
    ];
    let links = relation_links(&relations);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].kind, LinkKind::Relation);
    assert_eq!(links[0].relation_type, "supersedes");
}
