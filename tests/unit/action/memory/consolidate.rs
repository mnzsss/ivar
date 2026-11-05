#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use super::*;
use crate::domain::memory_page::PageMeta;
use crate::domain::session::rfc3339_from_epoch_secs;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;

const NOW: u64 = 1_800_000_000;

fn at(secs_ago: u64) -> String {
    rfc3339_from_epoch_secs((NOW - secs_ago) as f64)
}

fn plant(layout: &Layout, rows: &[(&str, &str, &str, u64, u8, &str)]) {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    let base: i64 = db
        .conn()
        .query_row("SELECT count(*) FROM observations", [], |r| r.get(0))
        .unwrap();
    for (n, (event, session, native, secs_ago, importance, body)) in rows.iter().enumerate() {
        db.conn()
            .execute(
                "INSERT INTO observations (event_id, source_event, occurred_at, session_id, native_session_id,
                  provider, ivar_version, origin, importance, body)
               VALUES (?1, ?2, ?3, ?4, ?5, 'claude-code', '0', 'user', ?6, ?7)",
                rusqlite::params![
                    format!("e{}", base + n as i64),
                    event,
                    at(*secs_ago),
                    session,
                    native,
                    importance,
                    body
                ],
            )
            .unwrap();
    }
}

fn hall() -> (tempfile::TempDir, Layout) {
    let (guard, root) = crate::test_support::seeded_hall();
    (guard, Layout::at(root))
}

fn page(layout: &Layout, native: &str) -> camino::Utf8PathBuf {
    layout
        .root()
        .join(crate::action::memory::page_file::page_rel(native))
}

#[test]
fn an_ended_conversation_gets_a_page_and_an_open_one_waits() {
    let (_g, layout) = hall();
    plant(
        &layout,
        &[
            ("session.prompted", "s1", "done", 600, 5, "fechar"),
            ("session.ended", "s1", "done", 500, 1, "{}"),
            ("session.prompted", "s1", "open", 60, 5, "ainda aberta"),
        ],
    );

    let report = consolidate_if_free(&layout, NOW).unwrap();

    assert_eq!(report.written, 1);
    assert!(page(&layout, "done").exists());
    assert!(!page(&layout, "open").exists());
}

#[test]
fn thirty_idle_minutes_close_a_conversation_without_an_end() {
    let (_g, layout) = hall();
    plant(
        &layout,
        &[("session.prompted", "", "idle", 31 * 60, 5, "ociosa")],
    );
    consolidate_if_free(&layout, NOW).unwrap();
    assert!(page(&layout, "idle").exists());
}

#[test]
fn a_second_pass_without_new_rows_leaves_the_file_alone() {
    let (_g, layout) = hall();
    plant(&layout, &[("session.prompted", "", "n", 3600, 5, "x")]);
    consolidate_if_free(&layout, NOW).unwrap();
    let before = std::fs::metadata(page(&layout, "n"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(consolidate_if_free(&layout, NOW).unwrap().written, 0);
    assert_eq!(
        std::fs::metadata(page(&layout, "n"))
            .unwrap()
            .modified()
            .unwrap(),
        before
    );
}

#[test]
fn a_new_row_regenerates_and_keeps_pinned_and_expires_at() {
    let (_g, layout) = hall();
    plant(
        &layout,
        &[("session.prompted", "", "n", 7200, 5, "primeiro")],
    );
    consolidate_if_free(&layout, NOW).unwrap();
    let path = page(&layout, "n");
    let text = std::fs::read_to_string(&path).unwrap().replacen(
        "---\n",
        "---\npinned: true\nexpires_at: \"2099-01-01T00:00:00Z\"\n",
        1,
    );
    std::fs::write(&path, text).unwrap();
    plant(
        &layout,
        &[(
            "session.tool",
            "",
            "n",
            3600,
            4,
            r#"{"tool":"Edit","input":{"file_path":"novo.rs"},"result":""}"#,
        )],
    );

    assert_eq!(consolidate_if_free(&layout, NOW).unwrap().written, 1);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("novo.rs"));
    assert!(text.contains("pinned: true"));
    assert!(text.contains("2099-01-01T00:00:00Z"));
}

#[test]
fn a_trivial_conversation_has_no_page_and_loses_an_old_one() {
    let (_g, layout) = hall();
    plant(&layout, &[("session.started", "", "t", 3600, 1, "{}")]);
    std::fs::create_dir_all(page(&layout, "t").parent().unwrap()).unwrap();
    std::fs::write(page(&layout, "t"), "---\ntitle: \"velha\"\n---\nx\n").unwrap();
    let report = consolidate_if_free(&layout, NOW).unwrap();
    assert_eq!(report.removed, 1);
    assert!(!page(&layout, "t").exists());
}

#[test]
fn a_delivery_event_lands_in_one_page_only() {
    let (_g, layout) = hall();
    plant(
        &layout,
        &[
            ("session.prompted", "s1", "a", 20_000, 5, "primeira"),
            ("session.ended", "s1", "a", 19_000, 1, "{}"),
            ("session.prompted", "s1", "b", 10_000, 5, "segunda"),
            (
                "delivery.pr",
                "s1",
                "",
                9_500,
                4,
                r#"{"url":"https://x/pr/9"}"#,
            ),
            ("session.ended", "s1", "b", 9_000, 1, "{}"),
        ],
    );
    consolidate_if_free(&layout, NOW).unwrap();
    assert!(
        !std::fs::read_to_string(page(&layout, "a"))
            .unwrap()
            .contains("delivery.pr")
    );
    assert!(
        std::fs::read_to_string(page(&layout, "b"))
            .unwrap()
            .contains("delivery.pr")
    );
}

#[test]
fn the_session_scope_closes_open_conversations_of_that_session_only() {
    let (_g, layout) = hall();
    plant(
        &layout,
        &[
            ("session.prompted", "s1", "mine", 60, 5, "minha"),
            ("session.prompted", "s2", "other", 60, 5, "outra"),
        ],
    );
    let report = consolidate_session(&layout, "s1", NOW).unwrap();
    assert_eq!(report.written, 1);
    assert!(page(&layout, "mine").exists());
    assert!(!page(&layout, "other").exists());
}

#[test]
fn a_held_lock_makes_the_free_pass_busy_and_writes_nothing() {
    let (_g, layout) = hall();
    plant(&layout, &[("session.prompted", "", "n", 3600, 5, "x")]);
    let _held = crate::infra::fs::lock_exclusive(&layout.memory_lock()).unwrap();
    assert!(consolidate_if_free(&layout, NOW).unwrap().busy);
    assert!(!page(&layout, "n").exists());
}

#[test]
fn without_a_memory_db_nothing_is_created() {
    let (_g, layout) = hall();
    let _ = std::fs::remove_file(layout.memory_db());
    assert_eq!(
        consolidate_if_free(&layout, NOW).unwrap(),
        ConsolidateReport::default()
    );
    assert!(!layout.memory_db().exists());
}

#[test]
fn written_pages_are_indexed() {
    let (_g, layout) = hall();
    plant(
        &layout,
        &[("session.prompted", "", "n", 3600, 5, "zebra unica")],
    );
    consolidate_if_free(&layout, NOW).unwrap();
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    let kind: String = db
        .conn()
        .query_row(
            "SELECT kind FROM pages WHERE path = ?1",
            [crate::action::memory::page_file::page_rel("n")],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kind, "session");
}

#[test]
fn expiry_follows_ttl_pinned_and_explicit_expires_at() {
    let now = NOW as i64;
    let old = now - 31 * 86_400;
    assert!(is_expired(&PageMeta::default(), old, now));
    assert!(!is_expired(
        &PageMeta {
            pinned: true,
            ..PageMeta::default()
        },
        old,
        now
    ));
    let past = PageMeta {
        pinned: true,
        expires_at: Some("2020-01-01T00:00:00Z".into()),
        ..PageMeta::default()
    };
    assert!(is_expired(&past, now, now));
    let future = PageMeta {
        expires_at: Some("2099-01-01T00:00:00Z".into()),
        ..PageMeta::default()
    };
    assert!(!is_expired(&future, old, now));
}

#[test]
fn a_marker_written_with_a_session_id_still_loads_and_is_rewritten_without_it() {
    let (_g, layout) = hall();
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    std::fs::write(
        layout.memory_spool_dir().join("_consolidate.json"),
        r#"{"markers": {"n": {"last_id": 7, "last_at": "2026-11-04T10:00:00Z", "session_id": "s1"}}}"#,
    )
    .unwrap();

    let loaded = state::read_state(&layout);
    assert_eq!(loaded.markers["n"].last_id, 7);
    state::write_state(&layout, &loaded).unwrap();

    let text =
        std::fs::read_to_string(layout.memory_spool_dir().join("_consolidate.json")).unwrap();
    assert!(!text.contains("session_id"), "{text}");
}
