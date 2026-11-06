#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use serde_json::{Value, json};

use crate::action::memory::mcp::{MemoryCaller, call_as};
use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName};
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

fn hall_with_feature(name: &str) -> (tempfile::TempDir, Layout) {
    let (guard, root) = seeded_hall();
    let layout = Layout::at(root);
    Feature::new(
        FeatureName::new(name).unwrap(),
        BranchName::new(name).unwrap(),
    )
    .write(&layout)
    .unwrap();
    (guard, layout)
}

fn session(id: &str, provider: &str, feature: Option<&str>) -> MemoryCaller {
    MemoryCaller {
        session: Some(id.to_owned()),
        feature: feature.map(|name| FeatureName::new(name).unwrap()),
        parent_feature: None,
        provider: provider.to_owned(),
        owner_key: id.to_owned(),
    }
}

fn outside(provider: &str, owner_key: &str) -> MemoryCaller {
    MemoryCaller {
        provider: provider.to_owned(),
        owner_key: owner_key.to_owned(),
        ..MemoryCaller::default()
    }
}

fn call(layout: &Layout, caller: &MemoryCaller, name: &str, args: Value) -> Value {
    serde_json::from_str(&call_as(layout, name, &args, caller).unwrap()).unwrap()
}

fn spooled_events(layout: &Layout) -> Vec<String> {
    std::fs::read_to_string(layout.memory_spool_dir().join("delivery.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line).unwrap()["source_event"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn begin_lands_in_the_sessions_feature_queue_and_only_that_feature_sees_it() {
    let (_guard, layout) = hall_with_feature("login");
    let writer = session("s1", "claude-code", Some("login"));

    let opened = call(
        &layout,
        &writer,
        "memory_handoff_begin",
        json!({
            "summary": "half the hook is wired",
            "pending_tasks": ["wire omp"],
            "modified_files": ["src/a.rs"]
        }),
    );

    assert!(opened["handoff_id"].as_str().is_some());
    assert!(opened["expires_at"].as_i64().unwrap() > 0);
    let same_feature = call(
        &layout,
        &session("s2", "omp", Some("login")),
        "memory_handoff_list",
        json!({}),
    );
    let discovery = call(
        &layout,
        &session("s3", "omp", None),
        "memory_handoff_list",
        json!({}),
    );
    assert_eq!(same_feature["handoffs"].as_array().unwrap().len(), 1);
    assert_eq!(
        same_feature["handoffs"][0]["pending_tasks"],
        json!(["wire omp"])
    );
    assert_eq!(same_feature["handoffs"][0]["owner"], "s1");
    assert_eq!(
        same_feature["handoffs"][0]["queue"],
        json!({"feature": "login"})
    );
    assert!(
        same_feature["notice"]
            .as_str()
            .unwrap()
            .contains("untrusted")
    );
    assert!(discovery["handoffs"].as_array().unwrap().is_empty());
    assert_eq!(spooled_events(&layout), ["handoff.opened"]);
}

#[test]
fn hall_true_and_an_explicit_feature_choose_the_queue_and_an_unknown_feature_is_refused() {
    let (_guard, layout) = hall_with_feature("login");
    let writer = session("s1", "claude-code", Some("login"));

    call(
        &layout,
        &writer,
        "memory_handoff_begin",
        json!({"summary": "x", "hall": true}),
    );
    let hall = call(
        &layout,
        &session("s9", "omp", None),
        "memory_handoff_list",
        json!({}),
    );
    let error = call_as(
        &layout,
        "memory_handoff_begin",
        &json!({"summary": "x", "feature": "nope"}),
        &writer,
    )
    .unwrap_err();
    let both = call_as(
        &layout,
        "memory_handoff_begin",
        &json!({"summary": "x", "feature": "login", "hall": true}),
        &writer,
    )
    .unwrap_err();

    assert_eq!(hall["handoffs"][0]["queue"], "hall");
    assert!(error.contains("nope"), "{error}");
    assert!(both.contains("either"), "{both}");
}

#[test]
fn an_addressed_handoff_is_seen_and_accepted_only_by_its_recipient() {
    let (_guard, layout) = hall_with_feature("login");
    let writer = session("s1", "claude-code", None);
    let opened = call(
        &layout,
        &writer,
        "memory_handoff_begin",
        json!({"summary": "for omp", "to": {"provider": "omp"}}),
    );
    let id = opened["handoff_id"].as_str().unwrap();

    let claude = call(
        &layout,
        &session("s2", "claude-code", None),
        "memory_handoff_accept",
        json!({"handoff_id": id}),
    );
    let omp = call(
        &layout,
        &outside("omp", "omp:mcp-1"),
        "memory_handoff_accept",
        json!({"handoff_id": id}),
    );
    let again = call(
        &layout,
        &outside("omp", "omp:mcp-2"),
        "memory_handoff_accept",
        json!({"handoff_id": id}),
    );

    assert_eq!(claude["status"], "none_pending");
    assert_eq!(omp["status"], "claimed");
    assert_eq!(omp["handoff"]["summary"], "for omp");
    assert!(omp["notice"].as_str().unwrap().contains("untrusted"));
    assert_eq!(again["status"], "already_claimed");
    assert_eq!(
        spooled_events(&layout),
        ["handoff.opened", "handoff.accepted"]
    );
}

#[test]
fn a_feature_filter_narrows_the_callers_own_view_and_never_widens_it() {
    let (_guard, layout) = hall_with_feature("login");
    Feature::new(
        FeatureName::new("billing").unwrap(),
        BranchName::new("billing").unwrap(),
    )
    .write(&layout)
    .unwrap();
    call(
        &layout,
        &session("s1", "claude-code", Some("billing")),
        "memory_handoff_begin",
        json!({"summary": "billing work"}),
    );
    call(
        &layout,
        &session("s1", "claude-code", Some("billing")),
        "memory_handoff_begin",
        json!({"summary": "for everyone", "hall": true}),
    );
    let login = session("s2", "omp", Some("login"));

    let billing = call(
        &layout,
        &login,
        "memory_handoff_list",
        json!({"feature": "billing"}),
    );
    let hall = call(
        &layout,
        &login,
        "memory_handoff_list",
        json!({"hall": true}),
    );
    let everything = call(&layout, &login, "memory_handoff_list", json!({}));

    assert_eq!(billing["handoffs"], json!([]));
    assert_eq!(hall["handoffs"].as_array().unwrap().len(), 1);
    assert_eq!(hall["handoffs"][0]["summary"], "for everyone");
    assert_eq!(everything["handoffs"].as_array().unwrap().len(), 1);
}

#[test]
fn list_clamps_its_limit_to_one_hundred() {
    let (_guard, layout) = hall_with_feature("login");
    let writer = session("s1", "claude-code", None);
    for i in 0..101 {
        call(
            &layout,
            &writer,
            "memory_handoff_begin",
            json!({"summary": format!("h{i}"), "hall": true}),
        );
    }

    let clamped = call(
        &layout,
        &writer,
        "memory_handoff_list",
        json!({"limit": 500}),
    );
    let default = call(&layout, &writer, "memory_handoff_list", json!({}));

    assert_eq!(clamped["handoffs"].as_array().unwrap().len(), 100);
    assert_eq!(default["handoffs"].as_array().unwrap().len(), 20);
}

#[test]
fn accepting_an_expired_handoff_by_id_reports_none_pending() {
    let (_guard, layout) = hall_with_feature("login");
    let writer = session("s1", "claude-code", None);
    let id = call(
        &layout,
        &writer,
        "memory_handoff_begin",
        json!({"summary": "stale", "hall": true}),
    )["handoff_id"]
        .as_str()
        .unwrap()
        .to_owned();
    crate::store::memory::db::MemoryDb::open(layout.memory_db().as_std_path())
        .unwrap()
        .conn()
        .execute("UPDATE handoffs SET expires_at = 1 WHERE id = ?1", [&id])
        .unwrap();

    let accepted = call(
        &layout,
        &session("s2", "omp", None),
        "memory_handoff_accept",
        json!({"handoff_id": id}),
    );

    assert_eq!(accepted["status"], "none_pending");
}

#[test]
fn the_serve_loop_owns_a_handoff_by_the_initialize_client_outside_a_session() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    let db = crate::store::graph::db::GraphDb::open_in_memory().unwrap();
    let input = format!(
        "{}\n{}\n",
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"clientInfo": {"name": "opencode"}}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "memory_handoff_begin", "arguments": {"summary": "from opencode", "hall": true}}})
    );
    let mut out = Vec::new();

    crate::action::graph::mcp::run_mcp_server(
        &db,
        Some(root.as_std_path()),
        std::io::Cursor::new(input),
        &mut out,
        |_| Ok(json!({"status": "ok"})),
    )
    .unwrap();

    let (key, provider): (String, String) =
        crate::store::memory::db::MemoryDb::open(layout.memory_db().as_std_path())
            .unwrap()
            .conn()
            .query_row(
                "SELECT owner_key, owner_provider FROM handoffs",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
    assert!(key.starts_with("opencode:mcp-"), "{key}");
    assert_eq!(provider, "opencode");
}

#[test]
fn accept_without_an_id_claims_the_newest_visible_handoff() {
    let (_guard, layout) = hall_with_feature("login");
    let writer = session("s1", "claude-code", None);
    call(
        &layout,
        &writer,
        "memory_handoff_begin",
        json!({"summary": "older", "hall": true}),
    );
    std::thread::sleep(std::time::Duration::from_millis(1100));
    call(
        &layout,
        &writer,
        "memory_handoff_begin",
        json!({"summary": "newer", "hall": true}),
    );

    let claimed = call(
        &layout,
        &session("s2", "omp", None),
        "memory_handoff_accept",
        json!({}),
    );
    let empty = call(
        &layout,
        &session("s3", "omp", None),
        "memory_handoff_accept",
        json!({}),
    );
    let last = call(
        &layout,
        &session("s4", "omp", None),
        "memory_handoff_accept",
        json!({}),
    );

    assert_eq!(claimed["handoff"]["summary"], "newer");
    assert_eq!(empty["handoff"]["summary"], "older");
    assert_eq!(last["status"], "none_pending");
}

#[test]
fn only_the_owner_cancels_through_the_tool() {
    let (_guard, layout) = hall_with_feature("login");
    let owner = outside("claude-code", "claude-code:mcp-a");
    let opened = call(
        &layout,
        &owner,
        "memory_handoff_begin",
        json!({"summary": "oops"}),
    );
    let id = opened["handoff_id"].as_str().unwrap();

    let refused = call_as(
        &layout,
        "memory_handoff_cancel",
        &json!({"handoff_id": id}),
        &outside("claude-code", "claude-code:mcp-b"),
    )
    .unwrap_err();
    let cancelled = call(
        &layout,
        &owner,
        "memory_handoff_cancel",
        json!({"handoff_id": id}),
    );

    assert!(refused.contains("not yours"), "{refused}");
    assert_eq!(cancelled["cancelled"], id);
    assert!(
        call(&layout, &owner, "memory_handoff_list", json!({}))["handoffs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        spooled_events(&layout),
        ["handoff.opened", "handoff.cancelled"]
    );
}

#[test]
fn begin_refuses_an_empty_summary_an_oversized_text_and_too_many_items() {
    let (_guard, layout) = hall_with_feature("login");
    let writer = session("s1", "claude-code", Some("login"));
    let refuse =
        |args: Value| call_as(&layout, "memory_handoff_begin", &args, &writer).unwrap_err();

    assert!(refuse(json!({"summary": ""})).contains("summary"));
    assert!(refuse(json!({"summary": "x".repeat(16 * 1024 + 1)})).contains("16384"));
    let many: Vec<String> = (0..51).map(|i| format!("t{i}")).collect();
    assert!(refuse(json!({"summary": "x", "pending_tasks": many})).contains("50"));
    assert!(
        refuse(json!({"summary": "x", "to": {"provider": "omp", "session": "s2"}})).contains("to")
    );
}

#[test]
fn begin_redacts_secrets_and_hall_env_values_from_the_stored_text() {
    let (_guard, layout) = hall_with_feature("login");
    std::fs::write(layout.root().join(".env"), "TOKEN=supersecretvalue123\n").unwrap();
    let writer = session("s1", "claude-code", Some("login"));

    call(
        &layout,
        &writer,
        "memory_handoff_begin",
        json!({"summary": "used supersecretvalue123 to log in"}),
    );

    let listed = call(&layout, &writer, "memory_handoff_list", json!({}));
    let summary = listed["handoffs"][0]["summary"].as_str().unwrap();
    assert!(!summary.contains("supersecretvalue123"), "{summary}");
}

#[test]
fn handoff_tools_are_listed_and_only_list_is_read_only() {
    let tools = crate::action::memory::mcp::tool_definitions();
    for name in [
        "memory_handoff_begin",
        "memory_handoff_list",
        "memory_handoff_accept",
        "memory_handoff_cancel",
    ] {
        let tool = tools.iter().find(|t| t["name"] == name).expect(name);
        assert!(crate::action::memory::mcp::is_memory_tool(name));
        assert_eq!(
            tool["annotations"]["readOnlyHint"],
            name == "memory_handoff_list",
            "{name}"
        );
    }
}
