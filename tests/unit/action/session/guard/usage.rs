//! Graph-call and search-miss recording from hook payloads.

use super::*;
use crate::domain::graph::{MissKind, UsageEvent, UsageSource};
use crate::store::graph::db::GraphDb;
use crate::store::graph::db::usage::MissFilter;

fn session_env_in_hall(
    root: &Utf8PathBuf,
    session_id: &str,
) -> crate::action::session::env::SessionEnv {
    let layout = Layout::at(root.clone());
    let session_id = SessionId::new(session_id).unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    crate::action::session::env::SessionEnv {
        hall: root.clone(),
        session_id: session_id.to_string(),
        view_dir,
        provider: Provider::ClaudeCode,
        feature: None,
    }
}

fn session_env_with_memory_db() -> (
    tempfile::TempDir,
    crate::action::session::env::SessionEnv,
    Utf8PathBuf,
) {
    let (guard, root) = hall_with_promoted_feature();
    let env = session_env_in_hall(&root, "6f0c9d5f-0000-4000-8000-0000000006ee");
    let db_path = Layout::at(root).ivar_dir().join("memory.db");
    GraphDb::open(db_path.as_std_path()).unwrap();
    (guard, env, db_path)
}

fn record_graph_call(db_path: &Utf8PathBuf, session: &str) {
    let db = GraphDb::open_for_usage(db_path.as_std_path()).unwrap();
    db.record_usage(&UsageEvent {
        command: "explore".to_owned(),
        source: UsageSource::Mcp,
        duration_ms: 5,
        result_count: Some(0),
        error: false,
        session: Some(session.to_owned()),
        query: Some("record_miss".to_owned()),
    })
    .unwrap();
}

fn all_misses(db_path: &Utf8PathBuf) -> Vec<crate::domain::graph::MissRecord> {
    GraphDb::open_for_usage(db_path.as_std_path())
        .unwrap()
        .list_misses(&MissFilter::default())
        .unwrap()
}

#[test]
fn a_search_with_no_prior_graph_call_is_recorded_as_skipped() {
    let (_guard, env, db_path) = session_env_with_memory_db();

    record_search_miss(
        &Layout::discover(&env.view_dir).unwrap().unwrap(),
        &env.session_id,
        "fn record_miss",
    );

    let misses = all_misses(&db_path);
    assert_eq!(misses.len(), 1);
    assert_eq!(misses[0].kind, MissKind::Skipped);
    assert_eq!(misses[0].session.as_deref(), Some(env.session_id.as_str()));
    assert_eq!(misses[0].pattern.as_deref(), Some("fn record_miss"));
}

#[test]
fn a_search_within_the_window_after_a_graph_call_is_recorded_as_followup() {
    let (_guard, env, db_path) = session_env_with_memory_db();
    record_graph_call(&db_path, &env.session_id);

    record_search_miss(
        &Layout::discover(&env.view_dir).unwrap().unwrap(),
        &env.session_id,
        "fn record_miss",
    );

    let misses = all_misses(&db_path);
    assert_eq!(misses.len(), 1);
    assert_eq!(misses[0].kind, MissKind::Followup);
    assert_eq!(misses[0].query.as_deref(), Some("record_miss"));
    assert_eq!(misses[0].pattern.as_deref(), Some("fn record_miss"));
}

#[test]
fn a_miss_recorded_in_the_same_second_before_a_graph_call_does_not_suppress_its_followup() {
    let (_guard, env, db_path) = session_env_with_memory_db();
    let layout = Layout::discover(&env.view_dir).unwrap().unwrap();
    record_search_miss(&layout, &env.session_id, "before the call");
    record_graph_call(&db_path, &env.session_id);
    let same_second = crate::store::graph::db::types::now_timestamp();
    GraphDb::open_for_usage(db_path.as_std_path())
        .unwrap()
        .conn()
        .execute_batch(&format!(
            "UPDATE graph_misses SET ts = {same_second}; UPDATE usage SET ts = {same_second};"
        ))
        .unwrap();

    record_search_miss(&layout, &env.session_id, "after the call");

    let misses = all_misses(&db_path);
    assert_eq!(misses.len(), 2);
    assert_eq!(misses[0].kind, MissKind::Followup);
    assert_eq!(misses[0].pattern.as_deref(), Some("after the call"));
}

#[test]
fn a_burst_of_greps_records_only_the_first_followup() {
    let (_guard, env, db_path) = session_env_with_memory_db();
    record_graph_call(&db_path, &env.session_id);

    record_search_miss(
        &Layout::discover(&env.view_dir).unwrap().unwrap(),
        &env.session_id,
        "first grep",
    );
    record_search_miss(
        &Layout::discover(&env.view_dir).unwrap().unwrap(),
        &env.session_id,
        "second grep",
    );
    record_search_miss(
        &Layout::discover(&env.view_dir).unwrap().unwrap(),
        &env.session_id,
        "third grep",
    );

    let misses = all_misses(&db_path);
    assert_eq!(
        misses.len(),
        1,
        "only the first search after the graph call is recorded"
    );
    assert_eq!(misses[0].pattern.as_deref(), Some("first grep"));
}

#[test]
fn guard_decision_is_unchanged_when_recording_fails() {
    let (_guard, root) = hall_with_promoted_feature();
    let env = session_env_in_hall(&root, "6f0c9d5f-0000-4000-8000-0000000006ff");
    let db_path = Layout::at(root).ivar_dir().join("memory.db");

    record_search_miss(
        &Layout::discover(&env.view_dir).unwrap().unwrap(),
        &env.session_id,
        "fn record_miss",
    );

    assert!(!db_path.exists());
    let req = ToolRequest {
        tool: "Grep".into(),
        targets: Vec::new(),
        writes: false,
        search_pattern: Some("fn record_miss".into()),
        input: serde_json::Value::Null,
        agent: None,
        call_id: None,
    };
    let set = resolve_writable_set(&env).unwrap();
    assert!(matches!(
        decide(&Resolution::Resolved(&set), &req, &[]),
        GuardDecision::Allow
    ));
}

#[test]
fn a_search_outside_any_session_is_keyed_by_the_ambient_session_id() {
    let (_guard, root) = hall_with_promoted_feature();
    let db_path = Layout::at(root.clone()).ivar_dir().join("memory.db");
    GraphDb::open(db_path.as_std_path()).unwrap();

    record_search_miss_at(
        &root,
        None,
        Some("ambient-session".to_owned()),
        "fn record_miss",
    );

    let misses = all_misses(&db_path);
    assert_eq!(misses.len(), 1);
    assert_eq!(misses[0].session.as_deref(), Some("ambient-session"));
}

#[test]
fn a_search_inside_a_resolved_session_is_keyed_by_that_session() {
    let (_guard, root) = hall_with_promoted_feature();
    let env = session_env_in_hall(&root, "6f0c9d5f-0000-4000-8000-0000000007aa");
    let db_path = Layout::at(root).ivar_dir().join("memory.db");
    GraphDb::open(db_path.as_std_path()).unwrap();

    record_search_miss_at(
        &env.view_dir,
        Some(&env),
        Some("ambient-session".to_owned()),
        "fn record_miss",
    );

    let misses = all_misses(&db_path);
    assert_eq!(misses.len(), 1);
    assert_eq!(misses[0].session.as_deref(), Some(env.session_id.as_str()));
}

#[test]
fn a_search_after_only_graph_feedback_is_recorded_as_skipped() {
    let (_guard, env, db_path) = session_env_with_memory_db();
    GraphDb::open_for_usage(db_path.as_std_path())
        .unwrap()
        .record_usage(&UsageEvent {
            command: "graph_feedback".to_owned(),
            source: UsageSource::Mcp,
            duration_ms: 1,
            result_count: None,
            error: false,
            session: Some(env.session_id.clone()),
            query: None,
        })
        .unwrap();

    record_search_miss(
        &Layout::discover(&env.view_dir).unwrap().unwrap(),
        &env.session_id,
        "fn record_miss",
    );

    let misses = all_misses(&db_path);
    assert_eq!(misses.len(), 1);
    assert_eq!(misses[0].kind, MissKind::Skipped);
}

#[test]
fn graph_explore_tool_names_are_recognised_across_providers() {
    assert!(is_graph_explore_tool("mcp__gaio-graph__graph_explore"));
    assert!(is_graph_explore_tool("gaio-graph_graph_explore"));
    assert!(!is_graph_explore_tool("mcp__gaio-graph__graph_feedback"));
    assert!(!is_graph_explore_tool("Grep"));
    assert!(is_graph_explore_tool("graph_explore"));
    assert!(is_graph_explore_tool("valhalla-hall-graph_graph_explore"));
    assert!(!is_graph_explore_tool("mcp__other__my_graph_explore"));
    assert!(!is_graph_explore_tool("foo_graph_explore"));
    assert!(!is_graph_explore_tool("foo_graph_explore_v2"));
}

#[test]
fn a_search_after_a_hook_recorded_graph_call_is_a_followup() {
    let (_guard, env, db_path) = session_env_with_memory_db();

    record_graph_call_at(&env.view_dir, Some(&env), None);
    record_search_miss(
        &Layout::discover(&env.view_dir).unwrap().unwrap(),
        &env.session_id,
        "fn record_miss",
    );

    let misses = all_misses(&db_path);
    assert_eq!(misses.len(), 1);
    assert_eq!(misses[0].kind, MissKind::Followup);
}

#[test]
fn a_hook_recorded_graph_call_outside_a_session_view_uses_the_ambient_session() {
    let (_guard, env, db_path) = session_env_with_memory_db();
    let hall = Layout::at(env.hall.clone());

    record_graph_call_at(hall.root(), None, Some("ambient-session".to_owned()));

    let db = GraphDb::open_for_usage(db_path.as_std_path()).unwrap();
    assert!(db.last_graph_call("ambient-session").unwrap().is_some());
}

fn hook_usage_rows(db_path: &Utf8PathBuf) -> Vec<(String, Option<String>)> {
    let db = GraphDb::open_for_usage(db_path.as_std_path()).unwrap();
    let mut stmt = db
        .conn()
        .prepare("SELECT source, session FROM usage WHERE command = 'graph_explore'")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn assert_hook_payload_records_graph_call(provider: Provider) {
    let (_guard, env, db_path) = session_env_with_memory_db();
    crate::domain::session::SessionState::new(provider, "2026-08-29T00:00:00Z")
        .write(&env.view_dir)
        .unwrap();
    let payload = serde_json::json!({
        "tool": "valhalla-hall-graph_graph_explore",
        "args": { "query": "record_graph_call_at" },
        "cwd": env.view_dir,
    });

    let out = guard(provider, &payload.to_string(), None).unwrap();

    assert!(out.exit_zero);
    assert_eq!(
        hook_usage_rows(&db_path),
        vec![("hook".to_owned(), Some(env.session_id.clone()))]
    );
}

#[test]
fn an_opencode_graph_explore_hook_payload_records_a_hook_usage_row() {
    assert_hook_payload_records_graph_call(Provider::OpenCode);
}

#[test]
fn an_omp_graph_explore_hook_payload_records_a_hook_usage_row() {
    assert_hook_payload_records_graph_call(Provider::Omp);
}
