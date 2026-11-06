#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::session::rfc3339_from_epoch_secs;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;

const NOW: u64 = 1_800_000_000;
const DAY: u64 = 86_400;

fn at(days_ago: u64) -> String {
    rfc3339_from_epoch_secs((NOW - days_ago * DAY) as f64)
}

fn plant(layout: &Layout, rows: &[(&str, &str, &str, u64)]) {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    for (n, (event, session, native, days_ago)) in rows.iter().enumerate() {
        db.conn()
            .execute(
                "INSERT INTO observations (event_id, source_event, occurred_at, session_id,
                    native_session_id, provider, ivar_version, importance, body)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'omp', '0', 1, '{}')",
                rusqlite::params![format!("e{n}"), event, at(*days_ago), session, native],
            )
            .unwrap();
    }
}

fn remaining(layout: &Layout) -> Vec<String> {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    let mut stmt = db
        .conn()
        .prepare("SELECT event_id FROM observations ORDER BY id")
        .unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn a_session_is_deleted_whole_once_its_last_observation_passes_thirty_days() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    plant(
        &layout,
        &[
            ("session.started", "gone", "n1", 400),
            ("session.tool", "gone", "n1", 31),
            ("session.started", "alive", "n2", 400),
            ("session.tool", "alive", "n2", 1),
        ],
    );

    let report = sweep(&layout, NOW).unwrap();

    assert_eq!(report.deleted, 2);
    assert_eq!(remaining(&layout), vec!["e2", "e3"]);
}

#[test]
fn delivery_memory_and_handoff_rows_survive_however_old_they_are() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    plant(
        &layout,
        &[
            ("delivery.pushed", "", "", 400),
            ("delivery.merged", "", "", 400),
            ("memory.reviewed", "s", "n", 400),
            ("handoff.opened", "s", "n", 400),
            ("session.ended", "s", "n", 400),
        ],
    );

    let report = sweep(&layout, NOW).unwrap();

    assert_eq!(report.deleted, 1);
    assert_eq!(remaining(&layout), vec!["e0", "e1", "e2", "e3"]);
}

#[test]
fn a_large_backlog_is_deleted_across_batches() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let rows: Vec<_> = (0..2500)
        .map(|_| ("session.tool", "old", "n", 90_u64))
        .collect();
    plant(&layout, &rows);

    assert_eq!(sweep(&layout, NOW).unwrap().deleted, 2500);
    assert!(remaining(&layout).is_empty());
}

#[test]
fn sweeping_a_hall_without_a_database_deletes_nothing_and_creates_none() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);

    let report = sweep(&layout, 4_000_000_000).unwrap();

    assert_eq!(report.deleted, 0);
    assert!(!layout.memory_db().exists());
}

#[test]
fn a_busy_memory_lock_skips_the_sweep_without_waiting() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    plant(&layout, &[("session.tool", "old", "n", 90)]);
    let _held = crate::infra::fs::lock_exclusive(&layout.memory_lock()).unwrap();

    let report = sweep(&layout, NOW).unwrap();

    assert!(report.busy);
    assert_eq!(report.deleted, 0);
    assert_eq!(remaining(&layout).len(), 1);
}

#[test]
fn the_state_file_accumulates_totals_and_records_a_failure() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    plant(
        &layout,
        &[
            ("session.tool", "a", "n", 90),
            ("session.tool", "a", "n", 91),
        ],
    );
    sweep(&layout, NOW).unwrap();
    plant_more(&layout);
    sweep(&layout, NOW).unwrap();
    let state = read_state(&layout);
    assert_eq!(state.swept_total, 3);
    assert!(state.last_sweep_at.is_some());
    assert_eq!(state.last_error, None);

    std::fs::write(layout.memory_db(), b"not a database").unwrap();
    assert!(sweep(&layout, NOW).is_err());
    assert!(read_state(&layout).last_error.is_some());
}

fn plant_more(layout: &Layout) {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    db.conn()
        .execute(
            "INSERT INTO observations (event_id, source_event, occurred_at, session_id,
                native_session_id, provider, ivar_version, importance, body)
             VALUES ('late', 'session.tool', ?1, 'b', 'n', 'omp', '0', 1, '{}')",
            [at(45)],
        )
        .unwrap();
}

#[test]
fn a_lock_that_cannot_be_taken_is_recorded_as_the_last_error() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.memory_lock()).unwrap();

    assert!(sweep(&layout, NOW).is_err());

    let state = read_state(&layout);
    assert!(state.last_error.is_some());
    assert_eq!(state.last_sweep_at, None);
}

#[test]
fn a_busy_lock_leaves_no_error_and_no_state() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let _held = crate::infra::fs::lock_exclusive(&layout.memory_lock()).unwrap();

    assert!(sweep(&layout, NOW).unwrap().busy);

    assert_eq!(read_state(&layout), SweepState::default());
}

fn session_page(layout: &Layout, name: &str, frontmatter: &str) -> camino::Utf8PathBuf {
    let dir = layout.root().join(".ivar/memory/session");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.md"));
    std::fs::write(
        &path,
        format!("---\ntitle: \"{name}\"\n{frontmatter}---\nx\n"),
    )
    .unwrap();
    path
}

fn mark(layout: &Layout, name: &str, days_ago: u64) {
    let mut state = crate::action::memory::consolidate::state::read_state(layout);
    state.markers.insert(
        name.to_owned(),
        crate::action::memory::consolidate::state::Marker {
            last_id: 1,
            last_at: at(days_ago),
        },
    );
    crate::action::memory::consolidate::state::write_state(layout, &state).unwrap();
}

#[test]
fn session_pages_past_thirty_days_are_removed_unless_pinned() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let old = session_page(&layout, "old", "");
    let pinned = session_page(&layout, "pinned", "pinned: true\n");
    let fresh = session_page(&layout, "fresh", "");
    mark(&layout, "old", 31);
    mark(&layout, "pinned", 31);
    mark(&layout, "fresh", 2);
    crate::action::memory::reindex::refresh(&layout).unwrap();

    let report = sweep(&layout, NOW).unwrap();

    assert_eq!(report.pages_removed, 1);
    assert!(!old.exists());
    assert!(pinned.exists());
    assert!(fresh.exists());
    assert!(
        crate::action::memory::consolidate::state::read_state(&layout)
            .markers
            .contains_key("old")
    );
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    let indexed: i64 = db
        .conn()
        .query_row(
            "SELECT count(*) FROM pages WHERE path = '.ivar/memory/session/old.md'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(indexed, 0);
}

#[test]
fn an_explicit_expires_at_wins_over_pinned() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let page = session_page(
        &layout,
        "gone",
        "pinned: true\nexpires_at: \"2020-01-01T00:00:00Z\"\n",
    );
    mark(&layout, "gone", 1);
    sweep(&layout, NOW).unwrap();
    assert!(!page.exists());
}

#[test]
fn a_page_without_a_marker_ages_by_its_mtime() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let page = session_page(&layout, "nomark", "");
    let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(NOW - 40 * DAY);
    std::fs::File::options()
        .write(true)
        .open(&page)
        .unwrap()
        .set_modified(old)
        .unwrap();
    sweep(&layout, NOW).unwrap();
    assert!(!page.exists());
}

#[test]
fn a_page_with_broken_frontmatter_is_left_alone() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let dir = layout.root().join(".ivar/memory/session");
    std::fs::create_dir_all(&dir).unwrap();
    let page = dir.join("broken.md");
    std::fs::write(&page, "---\npinned: [unclosed\n---\nx\n").unwrap();
    mark(&layout, "broken", 31);
    sweep(&layout, NOW).unwrap();
    assert!(page.exists());
}

#[test]
fn a_page_expired_by_its_expires_at_comes_back_only_with_a_new_observation() {
    use crate::action::memory::consolidate::consolidate_if_free;
    use crate::action::memory::page_file::page_rel;
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    for (id, event, importance, body) in [
        ("p", "session.prompted", 5, "fechar"),
        ("q", "session.ended", 1, "{}"),
    ] {
        db.conn()
            .execute(
                "INSERT INTO observations (event_id, source_event, occurred_at, session_id,
                    native_session_id, provider, ivar_version, origin, importance, body)
                 VALUES (?1, ?2, ?3, 's1', 'done', 'claude-code', '0', 'user', ?4, ?5)",
                rusqlite::params![id, event, at(1), importance, body],
            )
            .unwrap();
    }
    drop(db);
    let rel = page_rel("done");
    let page = layout.root().join(&rel);
    consolidate_if_free(&layout, NOW).unwrap();
    assert!(page.exists());
    crate::action::memory::mcp::call(
        &layout,
        "memory_patch_page",
        &serde_json::json!({ "path": rel, "expires_at": "2027-01-15T07:00:00Z" }),
    )
    .unwrap();

    assert_eq!(sweep(&layout, NOW).unwrap().pages_removed, 1);
    consolidate_if_free(&layout, NOW).unwrap();

    assert!(!page.exists());

    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    for (id, event, importance, body) in [
        ("r", "session.prompted", 5, "voltar"),
        ("s", "session.ended", 1, "{}"),
    ] {
        db.conn()
            .execute(
                "INSERT INTO observations (event_id, source_event, occurred_at, session_id,
                    native_session_id, provider, ivar_version, origin, importance, body)
                 VALUES (?1, ?2, ?3, 's1', 'done', 'claude-code', '0', 'user', ?4, ?5)",
                rusqlite::params![id, event, at(0), importance, body],
            )
            .unwrap();
    }
    drop(db);
    consolidate_if_free(&layout, NOW).unwrap();

    let recreated = std::fs::read_to_string(&page).expect("a new observation recreates the page");
    assert!(recreated.contains("voltar"), "{recreated}");
    assert!(!recreated.contains("expires_at"), "{recreated}");
}

#[test]
fn usage_and_misses_older_than_thirty_days_are_pruned() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    let old = (NOW - 31 * DAY) as i64;
    let recent = (NOW - DAY) as i64;
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO memory_usage (ts, source) VALUES ({old}, 'cli'), ({recent}, 'mcp');
             INSERT INTO memory_misses (ts, kind) VALUES ({old}, 'empty'), ({recent}, 'skipped');"
        ))
        .unwrap();
    drop(db);

    let report = sweep(&layout, NOW).unwrap();

    assert_eq!(report.usage_pruned, 2);
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    let left: i64 = db
        .conn()
        .query_row(
            "SELECT (SELECT count(*) FROM memory_usage) + (SELECT count(*) FROM memory_misses)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(left, 2);
}

use crate::domain::coordination::{NewHandoff, NewMessage, Owner, Queue};
use crate::store::memory::coordination::{insert_handoff, insert_message};

fn owner() -> Owner {
    Owner {
        key: "s-owner".to_owned(),
        provider: "claude-code".to_owned(),
    }
}

fn plant_handoff(layout: &Layout, id: &str, expires_days_ago: i64) {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    let expires_at = NOW as i64 - expires_days_ago * DAY as i64;
    insert_handoff(
        db.conn(),
        &NewHandoff {
            id: id.to_owned(),
            queue: Queue::Hall,
            address: None,
            owner: owner(),
            summary: "s".to_owned(),
            pending_tasks: vec![],
            open_decisions: vec![],
            modified_files: vec![],
            created_at: expires_at - 7 * DAY as i64,
            expires_at,
        },
    )
    .unwrap();
}

fn handoff_rows(layout: &Layout) -> Vec<(String, String)> {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    let mut stmt = db
        .conn()
        .prepare("SELECT id, state FROM handoffs ORDER BY id")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn overdue_handoffs_and_messages_expire_and_each_handoff_records_an_observation() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    plant_handoff(&layout, "h-due", 2);
    plant_handoff(&layout, "h-live", -3);
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    insert_message(
        db.conn(),
        &NewMessage {
            id: "m-due".to_owned(),
            queue: Queue::Hall,
            address: None,
            sender: owner(),
            subject: None,
            body: "b".to_owned(),
            created_at: NOW as i64 - 9 * DAY as i64,
            expires_at: NOW as i64 - 2 * DAY as i64,
        },
    )
    .unwrap();
    drop(db);

    let report = sweep(&layout, NOW).unwrap();

    assert_eq!(report.handoffs_expired, 1);
    assert_eq!(report.messages_expired, 1);
    assert_eq!(
        handoff_rows(&layout),
        vec![
            ("h-due".to_owned(), "expired".to_owned()),
            ("h-live".to_owned(), "open".to_owned())
        ]
    );
    let spool = std::fs::read_to_string(layout.memory_spool_dir().join("delivery.jsonl")).unwrap();
    let expired: Vec<serde_json::Value> = spool
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|l| l["source_event"] == "handoff.expired")
        .collect();
    assert_eq!(expired.len(), 1);
    assert!(expired[0]["body"].as_str().unwrap().contains("h-due"));
}

#[test]
fn terminal_rows_older_than_thirty_days_are_pruned_and_younger_ones_stay() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    plant_handoff(&layout, "h-ancient", 40);
    plant_handoff(&layout, "h-recent", 10);

    let report = sweep(&layout, NOW).unwrap();

    assert_eq!(report.coordination_pruned, 1);
    assert_eq!(
        handoff_rows(&layout),
        vec![("h-recent".to_owned(), "expired".to_owned())]
    );
}

#[test]
fn a_busy_sweep_expires_nothing() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    plant_handoff(&layout, "h-due", 2);
    let _held = crate::infra::fs::lock_exclusive(&layout.memory_lock()).unwrap();

    let report = sweep(&layout, NOW).unwrap();

    assert!(report.busy);
    assert_eq!(report.handoffs_expired, 0);
    assert_eq!(
        handoff_rows(&layout),
        vec![("h-due".to_owned(), "open".to_owned())]
    );
}

fn plant_fingerprint(layout: &Layout, label: &str, days_ago: u64) {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    db.conn()
        .execute(
            "INSERT INTO request_fingerprints (observation_id, day, normalized, created_at)
             VALUES (1, substr(?1, 1, 10), ?2, ?1)",
            rusqlite::params![at(days_ago), label],
        )
        .unwrap();
}

fn fingerprint_labels(layout: &Layout) -> Vec<String> {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    let mut stmt = db
        .conn()
        .prepare("SELECT normalized FROM request_fingerprints ORDER BY id")
        .unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn fingerprints_older_than_ninety_days_are_removed_and_younger_ones_stay() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    plant(&layout, &[]);
    for (label, days_ago) in [
        ("old", 91),
        ("older", 400),
        ("edge", 90),
        ("young", 89),
        ("today", 0),
    ] {
        plant_fingerprint(&layout, label, days_ago);
    }

    let report = sweep(&layout, NOW).unwrap();

    assert_eq!(report.fingerprints_pruned, 2);
    assert_eq!(fingerprint_labels(&layout), ["edge", "young", "today"]);
    assert_eq!(report.deleted, 0, "observation retention is unchanged");
}

#[test]
fn a_fingerprint_outlives_the_observation_it_came_from() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    plant(&layout, &[("session.prompted", "gone", "n1", 40)]);
    plant_fingerprint(&layout, "kept", 40);

    let report = sweep(&layout, NOW).unwrap();

    assert_eq!(report.deleted, 1);
    assert!(remaining(&layout).is_empty());
    assert_eq!(fingerprint_labels(&layout), ["kept"]);
}
