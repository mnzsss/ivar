use super::*;

#[test]
fn usage_summary_groups_by_command_and_source() {
    let db = GraphDb::open_in_memory().unwrap();
    for ms in [10, 20, 30, 40, 100] {
        db.record_usage(&event(
            "explore",
            UsageSource::Cli,
            ms,
            Some(3),
            false,
            None,
            None,
        ))
        .unwrap();
    }
    db.record_usage(&event(
        "explore",
        UsageSource::Cli,
        5,
        Some(0),
        false,
        None,
        None,
    ))
    .unwrap();
    db.record_usage(&event(
        "explore",
        UsageSource::Mcp,
        7,
        None,
        true,
        None,
        None,
    ))
    .unwrap();

    let summary = db.usage_summary().unwrap();

    let cli = summary
        .iter()
        .find(|s| s.source == UsageSource::Cli)
        .unwrap();
    assert_eq!(cli.command, "explore");
    assert_eq!(cli.count, 6);
    assert_eq!(cli.empty_count, 1);
    assert_eq!(cli.error_count, 0);
    assert_eq!(cli.p50_ms, 20);
    assert_eq!(cli.p95_ms, 100);
    assert!(cli.last_used > 0);

    let mcp = summary
        .iter()
        .find(|s| s.source == UsageSource::Mcp)
        .unwrap();
    assert_eq!(mcp.count, 1);
    assert_eq!(mcp.empty_count, 0);
    assert_eq!(mcp.error_count, 1);
}

#[test]
fn usage_summary_is_empty_without_events() {
    let db = GraphDb::open_in_memory().unwrap();
    assert!(db.usage_summary().unwrap().is_empty());
}

#[test]
fn clean_all_removes_misses() {
    let db = GraphDb::open_in_memory().unwrap();
    db.record_miss(&MissEvent {
        session: Some("sess-1".to_owned()),
        kind: MissKind::Skipped,
        query: None,
        pattern: Some("rg foo".to_owned()),
        reason: None,
    })
    .unwrap();
    db.clean_all().unwrap();
    assert!(db.list_misses(&MissFilter::default()).unwrap().is_empty());
}

#[test]
fn clean_all_removes_usage() {
    let db = GraphDb::open_in_memory().unwrap();
    db.record_usage(&event(
        "find",
        UsageSource::Cli,
        1,
        Some(1),
        false,
        None,
        None,
    ))
    .unwrap();
    assert_eq!(db.clean_all().unwrap().usage_removed, 1);
    assert!(db.usage_summary().unwrap().is_empty());
}

#[test]
fn stats_include_recorded_usage() {
    let db = GraphDb::open_in_memory().unwrap();
    db.record_usage(&event(
        "callers",
        UsageSource::Cli,
        4,
        Some(2),
        false,
        None,
        None,
    ))
    .unwrap();

    let stats = db.stats().unwrap();

    assert_eq!(stats.usage.len(), 1);
    assert_eq!(stats.usage[0].command, "callers");
}

#[test]
fn usage_summary_skips_rows_with_an_unknown_source() {
    let db = GraphDb::open_in_memory().unwrap();
    db.conn()
        .execute(
            "INSERT INTO usage (command, source, ts, duration_ms, result_count, error)
             VALUES ('find', 'web', 1, 1, 1, 0)",
            [],
        )
        .unwrap();
    assert!(db.usage_summary().unwrap().is_empty());
}

#[test]
fn opening_for_usage_never_creates_a_missing_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.db");
    assert!(GraphDb::open_for_usage(&path).is_err());
    assert!(!path.exists());
}

#[test]
fn opening_for_usage_rejects_an_unmigrated_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("PRAGMA user_version = 6;")
        .unwrap();
    assert!(GraphDb::open_for_usage(&path).is_err());
}

#[test]
fn opening_for_usage_records_into_a_migrated_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.db");
    drop(GraphDb::open(&path).unwrap());
    GraphDb::open_for_usage(&path)
        .unwrap()
        .record_usage(&event(
            "find",
            UsageSource::Cli,
            1,
            Some(1),
            false,
            None,
            None,
        ))
        .unwrap();
    assert_eq!(
        GraphDb::open(&path).unwrap().usage_summary().unwrap().len(),
        1
    );
}

#[test]
fn forget_repo_drops_the_repo_and_its_feature_layers_only() {
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("api", "/api", "main", None).unwrap();
    db.insert_repo("web", "/web", "main", None).unwrap();
    let api_layer = db
        .ensure_layer_record("feat", "api", "/f/api", "c1")
        .unwrap();
    let web_layer = db
        .ensure_layer_record("feat", "web", "/f/web", "c1")
        .unwrap();
    db.set_layer_tombstones(api_layer, &["a.rs"]).unwrap();
    db.insert_repo(&format!("api/{api_layer}"), "/f/api", "feat", None)
        .unwrap();

    db.forget_repo("api").unwrap();

    assert!(db.get_repo("api").unwrap().is_none());
    assert!(db.get_repo(&format!("api/{api_layer}")).unwrap().is_none());
    assert!(db.get_layer_record("feat", "api").unwrap().is_none());
    assert!(db.get_layer_tombstones(api_layer).unwrap().is_empty());
    assert!(db.get_repo("web").unwrap().is_some());
    assert_eq!(
        db.get_layer_record("feat", "web").unwrap().unwrap().id,
        web_layer
    );
}

#[test]
fn inserting_a_symbol_with_a_usize_id_beyond_i64_is_rejected_not_saturated() {
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("app", "/app", "main", None).unwrap();
    let file_id = db.upsert_file("app", "src/lib.rs", "h", 1, 1).unwrap();

    // SQLite stores the span as an i64 column: a usize::MAX span cannot round-trip,
    // so the out-of-range value must be rejected rather than silently clamped.
    let err = db
        .insert_symbols(&[Symbol {
            id: None,
            file_id: Some(file_id),
            repo: "app".to_owned(),
            name: "widget".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(usize::MAX, 1, usize::MAX, 2),
            is_exported: true,
            complexity: Some(u32::MAX),
        }])
        .expect_err("an out-of-range span must not be saturated");
    assert!(err.to_string().contains("too large") || err.to_string().contains("out of range"));
}

#[test]
fn record_usage_round_trips_session_and_query() {
    let db = GraphDb::open_in_memory().unwrap();
    db.record_usage(&UsageEvent {
        command: "explore".to_owned(),
        source: UsageSource::Cli,
        duration_ms: 3,
        result_count: Some(1),
        error: false,
        session: Some("sess-1".to_owned()),
        query: Some("enforceSession".to_owned()),
    })
    .unwrap();

    let (session, query): (Option<String>, Option<String>) = db
        .conn()
        .query_row("SELECT session, query FROM usage", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(session.as_deref(), Some("sess-1"));
    assert_eq!(query.as_deref(), Some("enforceSession"));
}

#[test]
fn record_miss_and_list_misses_round_trip() {
    let db = GraphDb::open_in_memory().unwrap();
    db.record_miss(&MissEvent {
        session: Some("sess-1".to_owned()),
        kind: MissKind::Skipped,
        query: None,
        pattern: Some("enforceSession".to_owned()),
        reason: None,
    })
    .unwrap();
    db.record_miss(&MissEvent {
        session: Some("sess-1".to_owned()),
        kind: MissKind::Feedback,
        query: Some("who calls login".to_owned()),
        pattern: None,
        reason: Some("empty result".to_owned()),
    })
    .unwrap();

    let all = db.list_misses(&MissFilter::default()).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].kind, MissKind::Feedback, "newest first");
    assert_eq!(all[0].reason.as_deref(), Some("empty result"));
    assert!(all[0].id > all[1].id);

    let skipped_only = db
        .list_misses(&MissFilter {
            kind: Some(MissKind::Skipped),
            since: None,
            limit: None,
        })
        .unwrap();
    assert_eq!(skipped_only.len(), 1);
    assert_eq!(skipped_only[0].pattern.as_deref(), Some("enforceSession"));
}

#[test]
fn list_misses_honors_limit_and_ordering() {
    let db = GraphDb::open_in_memory().unwrap();
    for i in 1..=15 {
        db.record_miss(&MissEvent {
            session: Some(format!("sess-{i}")),
            kind: MissKind::Skipped,
            query: Some(format!("query-{i}")),
            pattern: Some(format!("pattern-{i}")),
            reason: None,
        })
        .unwrap();
    }

    // Default (limit None) returns all 15, newest first
    let all = db.list_misses(&MissFilter::default()).unwrap();
    assert_eq!(all.len(), 15);
    assert_eq!(all[0].session.as_deref(), Some("sess-15"));

    // Explicit limit of 5 returns newest 5
    let limited = db
        .list_misses(&MissFilter {
            limit: Some(5),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(limited.len(), 5);
    assert_eq!(limited[0].session.as_deref(), Some("sess-15"));
    assert_eq!(limited[4].session.as_deref(), Some("sess-11"));
}

#[test]
fn list_misses_rejects_an_unknown_stored_kind() {
    let db = GraphDb::open_in_memory().unwrap();
    db.conn()
        .execute(
            "INSERT INTO graph_misses (ts, kind) VALUES (1, 'bogus')",
            [],
        )
        .unwrap();
    assert!(db.list_misses(&MissFilter::default()).is_err());
}

#[test]
fn last_graph_call_and_has_followup_for_answer_the_guard_questions() {
    let db = GraphDb::open_in_memory().unwrap();
    assert_eq!(db.last_graph_call("sess-1").unwrap(), None);

    db.record_usage(&UsageEvent {
        command: "explore".to_owned(),
        source: UsageSource::Cli,
        duration_ms: 1,
        result_count: Some(2),
        error: false,
        session: Some("sess-1".to_owned()),
        query: Some("enforceSession".to_owned()),
    })
    .unwrap();

    let call = db.last_graph_call("sess-1").unwrap().unwrap();
    assert!(call.ts > 0);
    assert_eq!(call.query.as_deref(), Some("enforceSession"));
    assert!(!db.has_followup_for(&call).unwrap());

    db.record_miss(&MissEvent {
        session: Some("sess-1".to_owned()),
        kind: MissKind::Skipped,
        query: None,
        pattern: Some("rg enforceSession".to_owned()),
        reason: None,
    })
    .unwrap();
    assert!(!db.has_followup_for(&call).unwrap());

    db.record_followup(
        &MissEvent {
            session: Some("sess-1".to_owned()),
            kind: MissKind::Followup,
            query: Some("enforceSession".to_owned()),
            pattern: Some("rg enforceSession".to_owned()),
            reason: None,
        },
        &call,
    )
    .unwrap();
    assert!(db.has_followup_for(&call).unwrap());
}

#[test]
fn prune_deletes_old_misses_and_old_usage_rows() {
    let db = GraphDb::open_in_memory().unwrap();
    let old_ts = crate::store::graph::db::types::now_timestamp() - 31 * 24 * 60 * 60;
    db.conn()
        .execute(
            "INSERT INTO graph_misses (ts, session, kind, query, pattern, reason)
             VALUES (?1, 'sess-1', 'skipped', NULL, 'old pattern', NULL)",
            rusqlite::params![old_ts],
        )
        .unwrap();
    db.record_miss(&MissEvent {
        session: Some("sess-1".to_owned()),
        kind: MissKind::Skipped,
        query: None,
        pattern: Some("recent pattern".to_owned()),
        reason: None,
    })
    .unwrap();
    db.conn()
        .execute(
            "INSERT INTO usage (command, source, ts, duration_ms, result_count, error, session, query)
             VALUES ('explore', 'cli', ?1, 1, 1, 0, 'sess-1', 'old query text')",
            rusqlite::params![old_ts],
        )
        .unwrap();

    let removed = db.prune(30).unwrap();
    assert_eq!(removed, 1);

    let remaining = db.list_misses(&MissFilter::default()).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].pattern.as_deref(), Some("recent pattern"));

    let old_usage: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM usage WHERE ts = ?1",
            rusqlite::params![old_ts],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(old_usage, 0);
}

#[test]
fn usage_summary_leaves_out_hook_rows() {
    let db = GraphDb::open_in_memory().unwrap();
    db.record_usage(&UsageEvent {
        command: "graph_explore".to_owned(),
        source: UsageSource::Hook,
        duration_ms: 0,
        result_count: None,
        error: false,
        session: Some("s".to_owned()),
        query: None,
    })
    .unwrap();
    assert!(db.usage_summary().unwrap().is_empty());
    assert!(db.last_graph_call("s").unwrap().is_some());
}
