use super::*;

#[test]
fn a_feature_session_whose_layer_cannot_refresh_answers_with_an_error() {
    use crate::domain::feature::Feature;
    use crate::domain::name::{BranchName, FeatureName, RepoName};
    use crate::store::layout::Layout;

    let (db, temp) = setup_test_mcp_db();
    let root = camino::Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();
    let layout = Layout::at(root.clone());
    std::fs::create_dir_all(layout.features_dir()).unwrap();
    let branch = BranchName::new("add-auth").unwrap();
    let mut feature = Feature::new(FeatureName::new("add-auth").unwrap(), branch.clone());
    feature.promote(RepoName::new("core").unwrap());
    feature.write(&layout).unwrap();
    let worktree = layout.repo_worktree(&RepoName::new("core").unwrap(), &branch);
    std::fs::create_dir_all(&worktree).unwrap();

    let (text, is_error) = call_tool_at(&db, temp.path(), &worktree);

    assert!(is_error, "got: {text}");
    assert!(text.starts_with("Error:"), "got: {text}");
    assert!(text.contains("feature layer"), "got: {text}");
    assert!(text.contains("base graph index missing"), "got: {text}");
}

#[cfg(unix)]
#[test]
fn an_unresolvable_session_answers_with_an_error_instead_of_the_base_graph() {
    use crate::store::layout::Layout;
    use std::os::unix::fs::PermissionsExt;

    let (db, temp) = setup_test_mcp_db();
    let root = camino::Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();
    let repos_dir = Layout::at(root.clone()).repos_dir();
    std::fs::create_dir_all(&repos_dir).unwrap();
    std::fs::set_permissions(&repos_dir, std::fs::Permissions::from_mode(0o000)).unwrap();

    let (text, is_error) = call_tool_at(&db, temp.path(), &root);
    std::fs::set_permissions(&repos_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert!(is_error, "got: {text}");
    assert!(text.contains("session"), "got: {text}");
    let summary = db.usage_summary().unwrap();
    let explore = summary
        .iter()
        .find(|s| s.command == "graph_explore")
        .expect("failed refresh still records usage");
    assert_eq!(explore.error_count, 1);
}

#[test]
fn a_tool_call_records_its_query_and_session_alongside_the_result_count() {
    let (db, temp) = setup_test_mcp_db();
    let cwd = camino::Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": "graph_explore", "arguments": { "query": "main" } }
    });
    let resp = handle_json_rpc_at(
        &db,
        Some(temp.path()),
        &cwd,
        ToolSurface::All,
        &req,
        &mut |_| Ok(json!({"status": "ok"})),
        None,
    )
    .expect("response");
    assert!(!resp["result"]["isError"].as_bool().unwrap_or(false));

    let usage_row = db
        .conn()
        .query_row(
            "SELECT result_count, query, session FROM usage WHERE command = 'graph_explore'",
            [],
            |r| {
                Ok((
                    r.get::<_, Option<i64>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(usage_row.0, Some(2), "execute and helper both match `main`");
    assert_eq!(usage_row.1.as_deref(), Some("main"));
    assert_eq!(
        usage_row.2, None,
        "no ivar session view is active for this temp hall"
    );

    let summary = db.usage_summary().unwrap();
    let explore = summary
        .iter()
        .find(|s| s.command == "graph_explore")
        .expect("graph_explore usage recorded");
    assert_eq!(explore.count, 1);
    assert_eq!(explore.empty_count, 0);
}

#[test]
fn a_query_matching_nothing_is_recorded_as_an_mcp_empty() {
    let (db, temp) = setup_test_mcp_db();

    let (_text, is_error) = call_tool(
        &db,
        temp.path(),
        "graph_explore",
        json!({"query": "no_such_symbol_anywhere"}),
    );
    assert!(!is_error);

    let summary = db.usage_summary().unwrap();
    let explore = summary
        .iter()
        .find(|s| s.command == "graph_explore")
        .expect("graph_explore usage recorded");
    assert_eq!(explore.source, crate::domain::graph::UsageSource::Mcp);
    assert_eq!(explore.empty_count, 1);
}

#[test]
fn a_500_char_query_is_truncated_before_storage() {
    let (db, temp) = setup_test_mcp_db();

    call_tool(
        &db,
        temp.path(),
        "graph_explore",
        json!({"query": "x".repeat(600)}),
    );

    let stored: Option<String> = db
        .conn()
        .query_row(
            "SELECT query FROM usage WHERE command = 'graph_explore'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored.map(|s| s.len()), Some(500));
}

#[test]
fn graph_feedback_is_advertised_under_both_surfaces() {
    let names = |surface| -> Vec<String> {
        super::tools::list_tools(surface)
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_owned))
            .collect()
    };
    assert!(names(super::tools::ToolSurface::All).contains(&"graph_feedback".to_owned()));
    assert!(names(super::tools::ToolSurface::Explore).contains(&"graph_feedback".to_owned()));
}

#[test]
fn the_server_instructions_tell_the_agent_to_call_graph_feedback_on_a_miss() {
    let (db, temp) = setup_test_mcp_db();
    let input = format!(
        "{}\n",
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
    );
    let mut output = Vec::new();
    run_mcp_server(
        &db,
        Some(temp.path()),
        Cursor::new(input),
        &mut output,
        |_| Ok(json!({"status": "ok"})),
    )
    .expect("run server");
    let resp: Value = serde_json::from_str(String::from_utf8(output).unwrap().trim()).unwrap();
    let instructions = resp["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("graph_feedback"), "{instructions}");
}

#[test]
fn a_failing_tool_call_records_an_error() {
    let (db, temp) = setup_test_mcp_db();

    let (_text, is_error) = call_tool(&db, temp.path(), "no_such_tool", json!({}));
    assert!(is_error);

    let summary = db.usage_summary().unwrap();
    assert_eq!(
        summary
            .iter()
            .find(|s| s.command == "unknown")
            .unwrap()
            .error_count,
        1
    );
}

#[test]
fn an_unknown_tool_name_is_never_recorded_verbatim() {
    let (db, temp) = setup_test_mcp_db();

    call_tool(&db, temp.path(), "arbitrary client text", json!({}));

    let summary = db.usage_summary().unwrap();
    assert!(summary.iter().all(|s| s.command != "arbitrary client text"));
    assert_eq!(summary.iter().filter(|s| s.command == "unknown").count(), 1);
}

#[test]
fn a_discovery_session_call_records_the_ivar_session_id() {
    use crate::domain::name::SessionId;
    use crate::domain::provider::Provider;
    use crate::domain::session::SessionState;
    use crate::store::layout::Layout;

    let (db, _temp) = setup_test_mcp_db();
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-0000000007aa").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    SessionState::new(Provider::ClaudeCode, "2026-08-29T00:00:00Z")
        .write(&view_dir)
        .unwrap();

    let (text, is_error) = call_tool_at(&db, root.as_std_path(), &view_dir);
    assert!(!is_error, "got: {text}");

    let session: Option<String> = db
        .conn()
        .query_row(
            "SELECT session FROM usage WHERE command = 'graph_explore'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(session.as_deref(), Some(session_id.as_str()));
}

#[test]
fn a_grep_after_an_mcp_explore_in_the_same_session_is_a_followup_carrying_the_query() {
    use crate::domain::graph::MissKind;
    use crate::domain::name::SessionId;
    use crate::domain::provider::Provider;
    use crate::domain::session::SessionState;
    use crate::store::graph::db::usage::MissFilter;
    use crate::store::layout::Layout;

    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    let db = GraphDb::open(layout.graph_db().as_std_path()).unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-0000000009cc").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    SessionState::new(Provider::ClaudeCode, "2026-08-29T00:00:00Z")
        .write(&view_dir)
        .unwrap();

    let (text, is_error) = call_tool_at(&db, root.as_std_path(), &view_dir);
    assert!(!is_error, "got: {text}");

    let hook = json!({
        "tool_name": "Grep",
        "tool_input": { "pattern": "fn main" },
        "cwd": view_dir,
    });
    crate::action::session::guard::guard(Provider::ClaudeCode, &hook.to_string(), None).unwrap();

    let misses = db.list_misses(&MissFilter::default()).unwrap();
    assert_eq!(misses.len(), 1, "{misses:?}");
    assert_eq!(misses[0].kind, MissKind::Followup);
    assert_eq!(misses[0].session.as_deref(), Some(session_id.as_str()));
    assert_eq!(misses[0].query.as_deref(), Some("execute"));
    assert_eq!(misses[0].pattern.as_deref(), Some("fn main"));
}

#[test]
fn starting_the_server_prunes_misses_and_usage_past_retention() {
    let db = GraphDb::open_in_memory().expect("open db");
    let old_ts = crate::store::graph::db::types::now_timestamp() - 31 * 24 * 60 * 60;
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO graph_misses (ts, kind) VALUES ({old_ts}, 'skipped');
             INSERT INTO usage (command, source, ts, duration_ms, error)
             VALUES ('explore', 'mcp', {old_ts}, 1, 0);"
        ))
        .expect("seed old rows");

    run_mcp_server(&db, None, Cursor::new(""), &mut Vec::new(), |_| {
        Ok(Value::Null)
    })
    .expect("server runs");

    let remaining: i64 = db
        .conn()
        .query_row(
            "SELECT (SELECT COUNT(*) FROM graph_misses) + (SELECT COUNT(*) FROM usage)",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(remaining, 0);
}
