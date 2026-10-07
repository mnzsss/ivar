#![allow(clippy::unwrap_used)]

use super::*;

#[test]
fn parse_tool_request_fills_search_pattern_for_grep() {
    let json = r#"{
        "tool_name": "Grep",
        "tool_input": {"pattern": "fn guard"},
        "cwd": "/tmp"
    }"#;
    let (req, _cwd) = parse_tool_request(json).unwrap();
    assert_eq!(req.search_pattern.as_deref(), Some("fn guard"));
}

#[test]
fn parse_tool_request_fills_search_pattern_for_bash_rg() {
    let json = r#"{
        "tool_name": "Bash",
        "tool_input": {"command": "rg 'fn guard' src/"},
        "cwd": "/tmp"
    }"#;
    let (req, _cwd) = parse_tool_request(json).unwrap();
    assert_eq!(req.search_pattern.as_deref(), Some("rg 'fn guard' src/"));
}

#[test]
fn parse_tool_request_leaves_search_pattern_none_for_write() {
    let json = r#"{
        "tool_name": "Write",
        "tool_input": {"file_path": "/tmp/x.rs"},
        "cwd": "/tmp"
    }"#;
    let (req, _cwd) = parse_tool_request(json).unwrap();
    assert_eq!(req.search_pattern, None);
}

#[test]
fn parse_tool_request_keeps_input_subagent_and_call_id() {
    let json = r#"{
        "session_id": "sess-1",
        "agent_id": "agent-7",
        "tool_use_id": "toolu_01",
        "tool_name": "Bash",
        "tool_input": {"command": "cat api/README.md"},
        "cwd": "/tmp"
    }"#;
    let (req, _cwd) = parse_tool_request(json).unwrap();
    assert_eq!(
        req.input,
        serde_json::json!({"command": "cat api/README.md"})
    );
    assert_eq!(req.agent.as_deref(), Some("agent-7"));
    assert_eq!(req.call_id.as_deref(), Some("toolu_01"));
}

#[test]
fn parse_tool_request_keys_the_main_agent_by_session_id() {
    let json = r#"{
        "session_id": "sess-1",
        "tool_use_id": "toolu_02",
        "tool_name": "Read",
        "tool_input": {"file_path": "/tmp/x.rs"},
        "cwd": "/tmp"
    }"#;
    let (req, _cwd) = parse_tool_request(json).unwrap();
    assert_eq!(req.agent.as_deref(), Some("sess-1"));
    assert_eq!(req.call_id.as_deref(), Some("toolu_02"));
}

#[test]
fn parse_tool_request_without_ids_leaves_them_none() {
    let json = r#"{
        "tool_name": "Read",
        "tool_input": {"file_path": "/tmp/x.rs"},
        "cwd": "/tmp"
    }"#;
    let (req, _cwd) = parse_tool_request(json).unwrap();
    assert_eq!(req.agent, None);
    assert_eq!(req.call_id, None);
    assert_eq!(req.input, serde_json::json!({"file_path": "/tmp/x.rs"}));
}
