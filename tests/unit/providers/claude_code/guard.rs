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
