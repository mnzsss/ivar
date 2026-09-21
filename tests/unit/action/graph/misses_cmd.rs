#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use camino::Utf8PathBuf;
use tempfile::TempDir;

use crate::action::Ctx;
use crate::action::graph::{MissesInput, misses_cmd, stats_cmd};
use crate::domain::graph::{MissEvent, MissKind};
use crate::store::graph::db::GraphDb;

fn hall_with(kinds: &[MissKind]) -> (TempDir, Ctx) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("ivar.json"),
        r#"{"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[],"version":1}"#,
    )
    .unwrap();
    let db_path = dir.path().join(".ivar/memory.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let db = GraphDb::open(&db_path).unwrap();
    for &kind in kinds {
        db.record_miss(&MissEvent {
            session: Some("sess-1".to_owned()),
            kind,
            query: Some("explore foo".to_owned()),
            pattern: Some("rg foo".to_owned()),
            reason: None,
        })
        .unwrap();
    }
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    (dir, Ctx::new(root))
}

fn age_all_misses(ctx: &Ctx, days: i64) {
    let db = GraphDb::open(ctx.cwd.join(".ivar/memory.db").as_std_path()).unwrap();
    db.conn()
        .execute("UPDATE graph_misses SET ts = ts - ?1", [days * 86_400])
        .unwrap();
}

fn input(kind: Option<&str>, since: Option<&str>) -> MissesInput {
    MissesInput {
        kind: kind.map(str::to_owned),
        since: since.map(str::to_owned),
    }
}

#[test]
fn kind_filter_excludes_other_kinds() {
    let (_dir, ctx) = hall_with(&[MissKind::Skipped, MissKind::Followup]);
    let report = misses_cmd(&ctx, &input(Some("followup"), None)).unwrap();
    assert_eq!(report.value.misses.len(), 1);
    assert_eq!(report.value.misses[0].kind, MissKind::Followup);
}

#[test]
fn unknown_kind_is_rejected() {
    let (_dir, ctx) = hall_with(&[]);
    assert!(misses_cmd(&ctx, &input(Some("nope"), None)).is_err());
}

#[test]
fn since_accepts_relative_durations_and_timestamps() {
    let (_dir, ctx) = hall_with(&[MissKind::Skipped]);
    age_all_misses(&ctx, 2);
    assert!(
        misses_cmd(&ctx, &input(None, Some("1d")))
            .unwrap()
            .value
            .misses
            .is_empty()
    );
    assert_eq!(
        misses_cmd(&ctx, &input(None, Some("3d")))
            .unwrap()
            .value
            .misses
            .len(),
        1
    );
    assert_eq!(
        misses_cmd(&ctx, &input(None, Some("0")))
            .unwrap()
            .value
            .misses
            .len(),
        1
    );
    assert!(misses_cmd(&ctx, &input(None, Some("7x"))).is_err());
}

#[test]
fn misses_prunes_rows_older_than_thirty_days() {
    let (_dir, ctx) = hall_with(&[MissKind::Skipped]);
    age_all_misses(&ctx, 31);
    assert!(
        misses_cmd(&ctx, &input(None, None))
            .unwrap()
            .value
            .misses
            .is_empty()
    );
}

#[test]
fn stats_prunes_rows_older_than_thirty_days() {
    let (_dir, ctx) = hall_with(&[MissKind::Skipped]);
    age_all_misses(&ctx, 31);
    stats_cmd(&ctx).unwrap();
    let db = GraphDb::open(ctx.cwd.join(".ivar/memory.db").as_std_path()).unwrap();
    let remaining: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM graph_misses", [], |row| row.get(0))
        .unwrap();
    assert_eq!(remaining, 0);
}

#[test]
fn since_rejects_negative_and_non_ascii_values_without_panicking() {
    let (_dir, ctx) = hall_with(&[]);
    for raw in ["-5d", "-5", "5é", "é", "", "d"] {
        let err = misses_cmd(&ctx, &input(None, Some(raw))).unwrap_err();
        assert_eq!(err.code, "graph.since_invalid", "{raw}");
    }
}
