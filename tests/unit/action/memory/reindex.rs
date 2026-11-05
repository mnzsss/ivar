#![allow(clippy::unwrap_used)]

use super::*;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;
use crate::test_support::seeded_hall;

fn write_page(layout: &Layout, rel: &str, text: &str) {
    let path = layout.root().join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn count(layout: &Layout, sql: &str) -> i64 {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    db.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn an_incremental_refresh_indexes_changes_and_removals() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    write_page(&layout, "docs/memory/gotcha/a.md", "# A\nfirst\n");
    write_page(&layout, "docs/memory/update/001-b.md", "# B\n");
    write_page(&layout, "docs/memory/update/002-b.cleanup.json", "{}");
    write_page(&layout, "docs/memory/gotcha/sub/ignored.md", "# nested\n");

    let first = refresh(&layout).unwrap();
    assert_eq!((first.indexed, first.unchanged, first.removed), (2, 0, 0));
    assert_eq!(first.invalid, ["docs/memory/gotcha/sub/ignored.md"]);

    let second = refresh(&layout).unwrap();
    assert_eq!((second.indexed, second.unchanged), (0, 2));

    write_page(&layout, "docs/memory/gotcha/a.md", "# A\nsecond edit\n");
    std::fs::remove_file(layout.root().join("docs/memory/update/001-b.md")).unwrap();
    let third = refresh(&layout).unwrap();
    assert_eq!((third.indexed, third.removed), (1, 1));
    assert_eq!(count(&layout, "SELECT count(*) FROM pages"), 1);

    let again = rebuild(&layout).unwrap();
    let twice = rebuild(&layout).unwrap();
    assert_eq!(
        (again.indexed, again.removed),
        (twice.indexed, twice.removed)
    );
    assert_eq!(count(&layout, "SELECT count(*) FROM pages"), 1);
}

#[test]
fn a_full_rebuild_keeps_every_observation() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    write_page(&layout, "docs/memory/gotcha/a.md", "# A\n");
    refresh(&layout).unwrap();
    {
        let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
        db.conn()
            .execute(
                "INSERT INTO observations (event_id, source_event, occurred_at, native_session_id, provider, ivar_version, importance, body)
                 VALUES ('e1', 'delivery.pr', '2026-11-01T00:00:00Z', '', 'p', '0', 4, 'kept')",
                [],
            )
            .unwrap();
    }

    rebuild(&layout).unwrap();

    assert_eq!(count(&layout, "SELECT count(*) FROM observations"), 1);
    assert_eq!(count(&layout, "SELECT count(*) FROM pages"), 1);
}

#[test]
fn the_leader_variant_does_not_wait_for_the_lock() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let _held = crate::infra::fs::lock_exclusive(&layout.memory_lock()).unwrap();

    let report = refresh_if_free(&layout).unwrap();

    assert_eq!(
        (report.indexed, report.unchanged, report.removed),
        (0, 0, 0)
    );
}

#[test]
fn a_failure_is_recorded_in_the_state_file_and_a_success_clears_it() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    refresh(&layout).unwrap();
    assert!(read_state(&layout).last_reindex_at.is_some());
    assert!(read_state(&layout).last_error.is_none());

    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", layout.memory_db()));
    }
    std::fs::write(layout.memory_db(), b"not a database").unwrap();
    assert!(refresh(&layout).is_err());
    assert!(read_state(&layout).last_error.is_some());

    std::fs::remove_file(layout.memory_db()).unwrap();
    refresh(&layout).unwrap();
    assert!(read_state(&layout).last_error.is_none());
}

fn make_inserts_of_boom_pages_fail(db: &MemoryDb) {
    db.conn()
        .execute_batch(
            "CREATE TRIGGER refuse_boom BEFORE INSERT ON pages WHEN NEW.path LIKE '%boom%'
             BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();
}

#[test]
fn a_lock_failure_is_recorded_like_any_other_reindex_failure() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_lock()).unwrap();

    assert!(refresh(&layout).is_err());
    assert!(read_state(&layout).last_error.is_some());

    write_state(&layout, &ReindexState::default());
    assert!(refresh_if_free(&layout).is_err());
    assert!(read_state(&layout).last_error.is_some());

    write_state(&layout, &ReindexState::default());
    assert!(rebuild(&layout).is_err());
    assert!(read_state(&layout).last_error.is_some());
}

#[test]
fn a_pass_that_changes_nothing_leaves_the_state_file_alone() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    write_page(&layout, "docs/memory/gotcha/a.md", "# A\n");
    refresh(&layout).unwrap();
    let mut state = read_state(&layout);
    state.last_reindex_at = Some("2000-01-01T00:00:00Z".to_owned());
    write_state(&layout, &state);

    let idle = refresh(&layout).unwrap();

    assert_eq!((idle.indexed, idle.removed), (0, 0));
    assert_eq!(
        read_state(&layout).last_reindex_at.as_deref(),
        Some("2000-01-01T00:00:00Z")
    );

    write_page(&layout, "docs/memory/gotcha/b.md", "# B\n");
    refresh(&layout).unwrap();
    assert_ne!(
        read_state(&layout).last_reindex_at.as_deref(),
        Some("2000-01-01T00:00:00Z")
    );
}

#[test]
fn a_link_resolves_once_its_target_page_is_added_later() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    write_page(&layout, "docs/memory/gotcha/a.md", "# A\nsee [[b]]\n");
    refresh(&layout).unwrap();
    let resolved = |layout: &Layout| {
        let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
        db.conn()
            .query_row("SELECT to_path FROM page_links", [], |r| {
                r.get::<_, Option<String>>(0)
            })
            .unwrap()
    };
    assert_eq!(resolved(&layout), None);

    refresh(&layout).unwrap();
    assert_eq!(resolved(&layout), None);

    write_page(&layout, "docs/memory/gotcha/b.md", "# B\n");
    refresh(&layout).unwrap();
    assert_eq!(
        resolved(&layout).as_deref(),
        Some("docs/memory/gotcha/b.md")
    );
}

#[test]
fn a_failed_rebuild_keeps_the_previous_index() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    write_page(&layout, "docs/memory/gotcha/a.md", "# A\nsee [[b]]\n");
    write_page(&layout, "docs/memory/gotcha/b.md", "# B\n");
    refresh(&layout).unwrap();
    write_page(&layout, "docs/memory/gotcha/boom.md", "# Boom\n");
    let mut db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    make_inserts_of_boom_pages_fail(&db);

    assert!(refresh_locked(&layout, &mut db, true).is_err());
    drop(db);

    assert_eq!(count(&layout, "SELECT count(*) FROM pages"), 2);
    assert_eq!(count(&layout, "SELECT count(*) FROM pages_fts"), 2);
    assert_eq!(
        count(
            &layout,
            "SELECT count(*) FROM page_links WHERE to_path IS NOT NULL"
        ),
        1
    );
    assert!(read_state(&layout).last_error.is_some());
}

#[test]
fn a_rebuild_reports_the_pages_that_no_longer_exist_as_removed() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    write_page(&layout, "docs/memory/gotcha/a.md", "# A\n");
    write_page(&layout, "docs/memory/gotcha/b.md", "# B\n");
    refresh(&layout).unwrap();
    std::fs::remove_file(layout.root().join("docs/memory/gotcha/b.md")).unwrap();

    let report = rebuild(&layout).unwrap();

    assert_eq!((report.indexed, report.removed), (1, 1));
}
