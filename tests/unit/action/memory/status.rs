use crate::action::Ctx;
use crate::action::memory::status::{status, status_in};
use crate::error::WriteHuman;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;
use crate::store::memory::schema::{APPLICATION_ID, MEMORY_SCHEMA_VERSION};
use crate::test_support::seeded_hall;

#[test]
fn status_repairs_a_memory_database_an_older_ivar_polluted() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    status(&Ctx::new(root.clone())).unwrap();
    drop(GraphDb::open(layout.memory_db().as_std_path()).unwrap());
    assert!(
        MemoryDb::inspect(layout.memory_db().as_std_path())
            .unwrap()
            .polluted
    );

    status(&Ctx::new(root)).unwrap();

    let inspection = MemoryDb::inspect(layout.memory_db().as_std_path()).unwrap();
    assert!(!inspection.polluted);
    assert_eq!(inspection.schema_version, MEMORY_SCHEMA_VERSION);
}

#[test]
fn status_moves_a_legacy_graph_and_reports_the_new_memory_database() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    drop(GraphDb::open(layout.memory_db().as_std_path()).unwrap());

    let report = status(&Ctx::new(root)).unwrap();

    let value = report.value;
    assert_eq!(value.path, layout.memory_db());
    assert!(value.exists);
    assert_eq!(value.schema_version, Some(MEMORY_SCHEMA_VERSION));
    assert_eq!(value.application_id, Some(APPLICATION_ID));
    assert!(value.size_bytes.unwrap() > 0);
    assert!(layout.graph_db().as_std_path().exists());
}

#[test]
fn status_json_carries_every_field() {
    let (_guard, root) = seeded_hall();

    let report = status(&Ctx::new(root)).unwrap();

    let json = serde_json::to_value(&report.value).unwrap();
    for field in [
        "path",
        "exists",
        "schema_version",
        "application_id",
        "size_bytes",
    ] {
        assert!(json.get(field).is_some(), "{field} missing from {json}");
    }
}

#[test]
fn status_waits_for_the_memory_lock_before_repairing() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    drop(GraphDb::open(layout.graph_db().as_std_path()).unwrap());
    status(&Ctx::new(root.clone())).unwrap();
    drop(GraphDb::open(layout.memory_db().as_std_path()).unwrap());
    let held = crate::infra::fs::lock_exclusive(&layout.memory_lock()).unwrap();
    let worker = std::thread::spawn(move || status(&Ctx::new(root)).is_ok());

    std::thread::sleep(std::time::Duration::from_secs(2));
    assert!(!worker.is_finished(), "status must wait for memory.lock");
    assert!(
        MemoryDb::inspect(layout.memory_db().as_std_path())
            .unwrap()
            .polluted,
        "status must not repair memory.db while memory.lock is held"
    );
    drop(held);

    assert!(worker.join().unwrap());
    assert!(
        !MemoryDb::inspect(layout.memory_db().as_std_path())
            .unwrap()
            .polluted
    );
}

#[test]
fn status_drains_the_spool_before_it_reads_and_reports_counts_and_spool_state() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    std::fs::create_dir_all(layout.memory_spool_dir()).unwrap();
    let event = serde_json::json!({
        "event_id": "a", "source_event": "session.started",
        "occurred_at": "2026-11-04T10:00:00.000000000Z", "session_id": "",
        "native_session_id": "n1", "provider": "claude-code", "ivar_version": "0.19.0",
        "feature": "", "parent_feature": "", "repo": "", "origin": "",
        "importance": 1, "attempt_id": "", "body": "{}"
    });
    std::fs::write(
        layout.memory_spool_dir().join("n1.jsonl"),
        format!("{event}\n"),
    )
    .unwrap();
    std::fs::write(layout.memory_spool_dir().join("n1.dropped"), "xx").unwrap();

    let value = status(&Ctx::new(root)).unwrap().value;

    assert_eq!(
        value
            .observations
            .iter()
            .map(|c| (c.source_event.as_str(), c.count))
            .collect::<Vec<_>>(),
        vec![("session.started", 1)]
    );
    assert_eq!(value.spool.files, 0, "drained");
    assert_eq!(value.spool.dropped, 0, "the drain acknowledged them");
    assert_eq!(value.spool.dropped_total, 2);
    assert!(value.last_drain_at.is_some());
    let json = serde_json::to_value(&value).unwrap();
    assert!(
        json.get("observations")
            .is_some_and(serde_json::Value::is_array)
    );
    assert_eq!(
        json.pointer("/spool/dropped_total"),
        Some(&serde_json::json!(2))
    );
}

#[test]
fn status_reports_the_retention_window_and_the_stalest_session() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    db.conn()
        .execute(
            "INSERT INTO observations (event_id, source_event, occurred_at, session_id,
                native_session_id, provider, ivar_version, importance, body)
             VALUES ('a', 'session.tool', '2026-01-02T00:00:00.000000000Z', 's', 'n', 'omp', '0', 1, '{}')",
            [],
        )
        .unwrap();
    drop(db);

    let report = status(&Ctx::new(root)).unwrap();

    assert_eq!(report.value.retention.window_days, 30);
    assert_eq!(
        report
            .value
            .retention
            .oldest_session_observation_at
            .as_deref(),
        Some("2026-01-02T00:00:00.000000000Z")
    );
    assert_eq!(report.value.retention.last_sweep_at, None);
    let json = serde_json::to_value(&report.value).unwrap();
    assert_eq!(
        json.pointer("/retention/window_days"),
        Some(&serde_json::json!(30))
    );
}

#[test]
fn status_reports_pages_per_kind_and_the_last_reindex() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    std::fs::create_dir_all(layout.root().join("docs/memory/gotcha")).unwrap();
    std::fs::write(layout.root().join("docs/memory/gotcha/a.md"), "# A\n").unwrap();
    std::fs::write(
        layout.root().join("docs/memory/gotcha/Bad_Slug.md"),
        "# B\n",
    )
    .unwrap();

    let value = status(&Ctx::new(root)).unwrap().value;

    assert_eq!(
        value
            .pages
            .by_kind
            .iter()
            .map(|k| (k.kind.as_str(), k.count))
            .collect::<Vec<_>>(),
        [("gotcha", 1)]
    );
    assert!(value.pages.last_reindex_at.is_some());
    assert_eq!(value.pages.invalid, 1);
}

#[test]
fn status_surfaces_a_failed_page_refresh_instead_of_reporting_zero_invalid() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    {
        let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
        db.conn().execute_batch("DROP TABLE pages_fts").unwrap();
    }
    std::fs::create_dir_all(layout.root().join("docs/memory/gotcha")).unwrap();
    std::fs::write(layout.root().join("docs/memory/gotcha/a.md"), "# A\n").unwrap();

    let value = status(&Ctx::new(root)).unwrap().value;

    assert!(value.pages.refresh_error.is_some());
    let mut human = Vec::new();
    value.write_human(&mut human).unwrap();
    assert!(
        String::from_utf8(human)
            .unwrap()
            .contains("page refresh failed")
    );
}

#[test]
fn status_in_reports_from_a_layout_without_a_ctx() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());

    let mut via_layout = status_in(&layout).unwrap().value;
    let mut via_ctx = status(&Ctx::new(root)).unwrap().value;
    via_layout.last_drain_at = None;
    via_ctx.last_drain_at = None;
    via_layout.consolidation.last_run_at = None;
    via_ctx.consolidation.last_run_at = None;

    assert_eq!(
        serde_json::to_value(&via_layout).unwrap(),
        serde_json::to_value(&via_ctx).unwrap()
    );
}

#[test]
fn status_reports_the_last_consolidation() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);

    let outcome = status_in(&layout).unwrap().value;

    assert!(outcome.consolidation.last_run_at.is_some());
    assert_eq!(outcome.consolidation.written_total, 0);
}

#[test]
fn status_counts_pages_waiting_for_anchors() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let _ = std::fs::remove_file(layout.graph_db());
    let page = layout.root().join("docs/memory/gotcha/a.md");
    std::fs::create_dir_all(page.parent().unwrap()).unwrap();
    std::fs::write(&page, "# A\n\nSee `src/a.rs`.\n").unwrap();

    let value = status_in(&layout).unwrap().value;

    assert_eq!(value.pages.anchors_pending, 1);
}
