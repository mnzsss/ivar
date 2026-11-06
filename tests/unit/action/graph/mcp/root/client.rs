use super::*;
use crate::action::graph::mcp::client::track_client;

#[test]
fn initialize_maps_the_client_name_through_the_provider_matrix() {
    let instance = uuid::Uuid::new_v4();
    let claude = McpClient::from_initialize(
        &json!({"clientInfo": {"name": "claude-code", "version": "2.1.0"}}),
        instance,
    );
    let opencode =
        McpClient::from_initialize(&json!({"clientInfo": {"name": "opencode"}}), instance);
    let omp = McpClient::from_initialize(
        &json!({"clientInfo": {"name": "omp-coding-agent"}}),
        instance,
    );
    let stranger = McpClient::from_initialize(&json!({"clientInfo": {"name": "zed"}}), instance);
    let bare = McpClient::from_initialize(&json!({}), instance);

    assert_eq!(claude.provider, "claude-code");
    assert_eq!(opencode.provider, "opencode");
    assert_eq!(omp.provider, "omp");
    assert_eq!(stranger.provider, "unknown");
    assert_eq!(bare.provider, "unknown");
    assert_eq!(claude.instance, instance);
    assert_eq!(
        claude.owner_key(),
        format!("claude-code:mcp-{}", claude.instance)
    );
}

#[test]
fn a_memory_call_outside_a_session_is_owned_by_the_server_instance() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = crate::store::layout::Layout::at(root.clone());
    let client = McpClient::from_initialize(
        &json!({"clientInfo": {"name": "opencode"}}),
        uuid::Uuid::new_v4(),
    );

    let caller = crate::action::memory::mcp::caller_for(&layout, &root, Some(&client));

    assert_eq!(caller.session, None);
    assert_eq!(caller.provider, "opencode");
    assert_eq!(caller.owner_key, client.owner_key());
}

#[test]
fn a_memory_call_with_no_client_and_no_session_has_an_unknown_owner() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = crate::store::layout::Layout::at(root.clone());

    let caller = crate::action::memory::mcp::caller_for(&layout, &root, None);

    assert_eq!(caller.provider, "unknown");
    assert_eq!(caller.owner_key, "unknown:mcp");
}

#[test]
fn initialize_sets_the_client_and_later_requests_keep_it_with_one_instance() {
    let instance = uuid::Uuid::new_v4();
    let mut client = None;

    track_client(&mut client, &json!({"method": "tools/list"}), instance);
    assert_eq!(client, None);
    track_client(
        &mut client,
        &json!({"method": "initialize", "params": {"clientInfo": {"name": "opencode"}}}),
        instance,
    );
    track_client(
        &mut client,
        &json!({"method": "tools/call", "params": {"name": "memory_status"}}),
        instance,
    );
    let kept = client.clone().unwrap();
    track_client(
        &mut client,
        &json!({"method": "initialize", "params": {"clientInfo": {"name": "claude-code"}}}),
        instance,
    );

    assert_eq!(kept.provider, "opencode");
    assert_eq!(kept.instance, instance);
    assert_eq!(
        client.unwrap().instance,
        instance,
        "a re-initialize keeps the process instance"
    );
}

#[test]
fn the_serve_loop_keeps_the_initialize_client_for_later_calls() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let db = GraphDb::open_in_memory().unwrap();
    let input = format!(
        "{}\n{}\n",
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"clientInfo": {"name": "claude-code"}}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "memory_status", "arguments": {}}})
    );
    let mut out = Vec::new();

    run_mcp_server(
        &db,
        Some(root.as_std_path()),
        std::io::Cursor::new(input),
        &mut out,
        |_| Ok(json!({"status": "ok"})),
    )
    .unwrap();

    let lines: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert!(
        lines[1]["result"]["isError"].as_bool() != Some(true),
        "{}",
        lines[1]
    );
}
