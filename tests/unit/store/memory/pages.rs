#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::*;
use crate::domain::memory_page::PageKind;
use crate::domain::memory_page::links::{extract_links, relation_links};
use crate::store::memory::db::MemoryDb;
use crate::store::memory::page_source::parse_page;

fn put(db: &mut MemoryDb, path: &str, source: &str) {
    let page = parse_page(path, source).unwrap();
    let mut links = extract_links(&page.body);
    links.extend(relation_links(&page.meta.relations));
    let file = IndexedFile {
        path: path.into(),
        size: source.len() as u64,
        mtime_ns: 1,
        content_hash: "h".into(),
    };
    upsert_page(db, &page, &file, "2026-11-05T00:00:00Z", &links).unwrap();
}

fn open() -> (tempfile::TempDir, MemoryDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDb::open(&dir.path().join("m.sqlite")).unwrap();
    (dir, db)
}

fn filter() -> PageFilter {
    PageFilter {
        kinds: vec![],
        tier: None,
        tag: None,
        include_expired: false,
        now: "2026-11-05T00:00:00Z".into(),
    }
}

#[test]
fn a_page_is_found_by_title_and_body_and_replaced_on_upsert() {
    let (_dir, mut db) = open();
    put(
        &mut db,
        "docs/memory/product/003-worktree-teardown.md",
        "# Worktree teardown\n\nremoves orphans\n",
    );
    assert_eq!(
        search_pages(&db, "\"orphans\"", &filter(), 10)
            .unwrap()
            .len(),
        1
    );
    put(
        &mut db,
        "docs/memory/product/003-worktree-teardown.md",
        "# Worktree teardown\n\nnow different\n",
    );
    assert!(
        search_pages(&db, "\"orphans\"", &filter(), 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        search_pages(&db, "\"different\"", &filter(), 10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn filters_narrow_by_kind_tag_and_expiry() {
    let (_dir, mut db) = open();
    put(
        &mut db,
        "docs/memory/gotcha/a.md",
        "---\ntags: [fts]\n---\n# A\nshared word\n",
    );
    put(
        &mut db,
        "docs/memory/decision/b.md",
        "---\nexpires_at: 2020-01-01T00:00:00Z\n---\n# B\nshared word\n",
    );
    assert_eq!(
        search_pages(&db, "\"shared\"", &filter(), 10)
            .unwrap()
            .len(),
        1
    );
    let mut with_expired = filter();
    with_expired.include_expired = true;
    assert_eq!(
        search_pages(&db, "\"shared\"", &with_expired, 10)
            .unwrap()
            .len(),
        2
    );
    let mut gotchas = with_expired.clone();
    gotchas.kinds = vec![PageKind::Gotcha];
    assert_eq!(
        search_pages(&db, "\"shared\"", &gotchas, 10).unwrap().len(),
        1
    );
    let mut tagged = with_expired;
    tagged.tag = Some("fts".into());
    assert_eq!(
        search_pages(&db, "\"shared\"", &tagged, 10).unwrap()[0].path,
        "docs/memory/gotcha/a.md"
    );
}

#[test]
fn links_resolve_by_slug_and_by_kind_and_slug_in_both_directions() {
    let (_dir, mut db) = open();
    put(
        &mut db,
        "docs/memory/gotcha/a.md",
        "# A\nsee [[b]] and [[concept/c]]\n",
    );
    put(&mut db, "docs/memory/decision/b.md", "# B\n");
    put(&mut db, "docs/memory/concept/c.md", "# C\n");
    resolve_links(&mut db).unwrap();
    let mut around_b = linked_paths(&db, &["docs/memory/decision/b.md".into()]).unwrap();
    around_b.sort();
    assert_eq!(around_b, ["docs/memory/gotcha/a.md"]);
    assert_eq!(
        linked_paths(&db, &["docs/memory/gotcha/a.md".into()])
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn a_relation_is_stored_with_its_type_and_resolves_to_the_target_page() {
    let (_dir, mut db) = open();
    put(
        &mut db,
        "docs/memory/decision/new.md",
        "---\nrelations:\n  - type: supersedes\n    target: decision/old\n---\n# New\n",
    );
    put(&mut db, "docs/memory/decision/old.md", "# Old\n");
    resolve_links(&mut db).unwrap();
    let (kind, to): (String, Option<String>) = db
        .conn()
        .query_row(
            "SELECT relation_type, to_path FROM page_links WHERE source = 'relation'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(kind, "supersedes");
    assert_eq!(to.as_deref(), Some("docs/memory/decision/old.md"));
}

#[test]
fn removing_a_page_removes_its_search_row_and_links() {
    let (_dir, mut db) = open();
    put(
        &mut db,
        "docs/memory/gotcha/a.md",
        "# A\nuniqueword [[b]]\n",
    );
    remove_page(&mut db, "docs/memory/gotcha/a.md").unwrap();
    assert!(
        search_pages(&db, "\"uniqueword\"", &filter(), 10)
            .unwrap()
            .is_empty()
    );
    let links: i64 = db
        .conn()
        .query_row("SELECT count(*) FROM page_links", [], |r| r.get(0))
        .unwrap();
    assert_eq!(links, 0);
}
