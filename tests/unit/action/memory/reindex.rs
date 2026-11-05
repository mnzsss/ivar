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
