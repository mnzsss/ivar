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
    let tx = db.conn_mut().transaction().unwrap();
    upsert_page(&tx, &page, &file, "2026-11-05T00:00:00Z", &links).unwrap();
    tx.commit().unwrap();
}

fn resolve(db: &mut MemoryDb) {
    let tx = db.conn_mut().transaction().unwrap();
    resolve_links(&tx).unwrap();
    tx.commit().unwrap();
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
    resolve(&mut db);
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
fn links_and_relations_resolve_to_session_pages_under_the_ephemeral_root() {
    let (_dir, mut db) = open();
    put(
        &mut db,
        "docs/memory/decision/d.md",
        "---\nrelations:\n  - type: derived_from\n    target: session/abc\n---\n# D\nsee [[session/abc]] and [[abc]]\n",
    );
    put(&mut db, ".ivar/memory/session/abc.md", "# Abc\n");
    resolve(&mut db);
    let resolved: Vec<Option<String>> = db
        .conn()
        .prepare("SELECT to_path FROM page_links ORDER BY rowid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(resolved.len(), 3);
    assert!(
        resolved
            .iter()
            .all(|to| to.as_deref() == Some(".ivar/memory/session/abc.md"))
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
    resolve(&mut db);
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
    let tx = db.conn_mut().transaction().unwrap();
    remove_page(&tx, "docs/memory/gotcha/a.md").unwrap();
    tx.commit().unwrap();
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

#[test]
fn resolving_links_again_writes_nothing() {
    let (_dir, mut db) = open();
    put(&mut db, "docs/memory/gotcha/a.md", "# A\nsee [[b]]\n");
    put(&mut db, "docs/memory/gotcha/b.md", "# B\n");
    resolve(&mut db);
    let before = db.conn().total_changes();

    resolve(&mut db);

    assert_eq!(db.conn().total_changes(), before);
}
