#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::*;
use crate::action::Ctx;
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

fn page(layout: &Layout, rel: &str, text: &str) {
    let path = layout.root().join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn input(text: &str) -> QueryInput {
    QueryInput {
        text: text.into(),
        kinds: vec![],
        tier: None,
        tag: None,
        limit: 10,
        include_expired: false,
        explain: true,
    }
}

#[test]
fn a_new_uncommitted_page_is_found_at_once() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    page(
        &layout,
        "docs/memory/gotcha/fts-shadow-tables.md",
        "# FTS shadow tables\n\nlist them in MEMORY_OBJECTS\n",
    );

    let outcome = query(&Ctx::new(root), &input("shadow tables"))
        .unwrap()
        .value;

    assert_eq!(
        outcome.pages[0].path,
        "docs/memory/gotcha/fts-shadow-tables.md"
    );
    assert!(outcome.observations.is_empty());
}

#[test]
fn a_linked_page_rides_along_without_matching_the_text() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    page(
        &layout,
        "docs/memory/gotcha/a.md",
        "# A\nabout teardown, see [[b]]\n",
    );
    page(
        &layout,
        "docs/memory/decision/b.md",
        "# B\nunrelated words\n",
    );

    let outcome = query(&Ctx::new(root), &input("teardown")).unwrap().value;

    let paths: Vec<_> = outcome.pages.iter().map(|h| h.path.as_str()).collect();
    assert_eq!(
        paths,
        ["docs/memory/gotcha/a.md", "docs/memory/decision/b.md"]
    );
    assert!(outcome.pages[1].sources.iter().any(|s| s.source == "links"));
}

#[test]
fn pinned_and_tier_adjust_the_order_of_equal_matches() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    page(
        &layout,
        "docs/memory/gotcha/plain.md",
        "# Same\nsame words here\n",
    );
    page(
        &layout,
        "docs/memory/decision/pinned.md",
        "---\npinned: true\n---\n# Same\nsame words here\n",
    );

    let outcome = query(&Ctx::new(root), &input("same words")).unwrap().value;

    assert_eq!(outcome.pages[0].path, "docs/memory/decision/pinned.md");
}

#[test]
fn observations_answer_only_when_no_page_matches_and_no_filter_is_set() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    page(&layout, "docs/memory/gotcha/a.md", "# A\nabout pages\n");
    {
        let db =
            crate::store::memory::db::MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
        db.conn()
            .execute(
                "INSERT INTO observations (event_id, source_event, occurred_at, native_session_id, provider, ivar_version, importance, body)
                 VALUES ('e1', 'delivery.pr', '2026-11-01T00:00:00Z', '', 'p', '0', 4, 'opened pull request 226')",
                [],
            )
            .unwrap();
    }

    let miss = query(&Ctx::new(root.clone()), &input("pull request"))
        .unwrap()
        .value;
    assert!(miss.pages.is_empty());
    assert_eq!(miss.observations[0].source_event, "delivery.pr");

    let mut filtered = input("pull request");
    filtered.kinds = vec!["gotcha".into()];
    assert!(
        query(&Ctx::new(root.clone()), &filtered)
            .unwrap()
            .value
            .observations
            .is_empty()
    );

    let hit = query(&Ctx::new(root), &input("pages")).unwrap().value;
    assert!(hit.observations.is_empty());
}

#[test]
fn an_unknown_kind_or_tier_is_refused_and_an_oversized_limit_is_capped() {
    let (_guard, root) = seeded_hall();
    let mut bad = input("x");
    bad.kinds = vec!["rule".into()];
    assert!(query(&Ctx::new(root.clone()), &bad).is_err());
    let mut bad_tier = input("x");
    bad_tier.tier = Some("eternal".into());
    assert_eq!(
        query(&Ctx::new(root.clone()), &bad_tier).unwrap_err().code,
        "memory.tier_unknown"
    );
    let mut big = input("x");
    big.limit = 500;
    assert!(query(&Ctx::new(root.clone()), &big).is_ok());
    for n in 0..25 {
        let path = root.join(format!("docs/memory/gotcha/many-{n:02}.md"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "# Many\ncommonword\n").unwrap();
    }
    let mut many = input("commonword");
    many.limit = 500;
    assert_eq!(query(&Ctx::new(root), &many).unwrap().value.pages.len(), 20);
}
