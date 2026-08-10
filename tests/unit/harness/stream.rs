//! Unit tests for `crate::harness::stream`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

// -- claude: envelopes vs. content blocks ---------------------------------

#[test]
fn a_tool_use_block_nested_under_message_content_is_found() {
    let line = r#"{"type":"assistant","message":{"content":[
        {"type":"tool_use","name":"Read","input":{"file_path":"src/lib.rs"}}
    ]}}"#;

    let events = parse_claude_line(line);

    assert_eq!(
        events,
        vec![ExecutorEvent::ToolUsed {
            tool: "Read".to_owned(),
            path: Some("src/lib.rs".to_owned()),
        }]
    );
}

/// The bug this module exists to avoid: `tool_use` never appears at the
/// top level, so matching the envelope's own `type` field against it must
/// match nothing.
#[test]
fn a_top_level_type_of_tool_use_matches_nothing() {
    let line = r#"{"type":"tool_use","name":"Read","input":{"file_path":"src/lib.rs"}}"#;

    assert_eq!(parse_claude_line(line), Vec::new());
}

#[test]
fn assistant_prose_produces_no_question() {
    let line = r#"{"type":"assistant","message":{"content":[
        {"type":"text","text":"I will start by reading the config file."}
    ]}}"#;

    assert_eq!(parse_claude_line(line), Vec::new());
}

#[test]
fn only_an_explicit_ask_user_question_call_is_a_question() {
    let line = r#"{"type":"assistant","message":{"content":[
        {"type":"tool_use","name":"AskUserQuestion","input":{"question":"Which port?"}}
    ]}}"#;

    assert_eq!(
        parse_claude_line(line),
        vec![ExecutorEvent::QuestionAsked {
            prompt: "Which port?".to_owned(),
        }]
    );
}

#[test]
fn ask_user_question_supports_the_questions_array_shape() {
    let line = r#"{"type":"assistant","message":{"content":[
        {"type":"tool_use","name":"AskUserQuestion","input":{"questions":[
            {"question":"Which port?"},
            {"question":"Which env?"}
        ]}}
    ]}}"#;

    assert_eq!(
        parse_claude_line(line),
        vec![ExecutorEvent::QuestionAsked {
            prompt: "Which port?\nWhich env?".to_owned(),
        }]
    );
}

#[test]
fn several_tool_calls_in_one_envelope_yield_several_events() {
    let line = r#"{"type":"assistant","message":{"content":[
        {"type":"tool_use","name":"Read","input":{"file_path":"a.rs"}},
        {"type":"tool_use","name":"Write","input":{"file_path":"b.rs"}},
        {"type":"tool_use","name":"Bash","input":{"command":"ls"}}
    ]}}"#;

    let events = parse_claude_line(line);

    assert_eq!(
        events,
        vec![
            ExecutorEvent::ToolUsed {
                tool: "Read".to_owned(),
                path: Some("a.rs".to_owned()),
            },
            ExecutorEvent::ToolUsed {
                tool: "Write".to_owned(),
                path: Some("b.rs".to_owned()),
            },
            ExecutorEvent::ToolUsed {
                tool: "Bash".to_owned(),
                path: None,
            },
        ]
    );
}

// -- claude: native session id ---------------------------------------------

#[test]
fn the_native_session_id_arrives_on_system_init() {
    let line = r#"{"type":"system","subtype":"init","session_id":"native-abc-123"}"#;

    assert_eq!(
        parse_claude_line(line),
        vec![ExecutorEvent::NativeSession {
            id: "native-abc-123".to_owned(),
        }]
    );
}

#[test]
fn a_system_envelope_of_a_different_subtype_carries_no_session_id() {
    let line = r#"{"type":"system","subtype":"other","session_id":"native-abc-123"}"#;

    assert_eq!(parse_claude_line(line), Vec::new());
}

// -- claude: malformed and irrelevant lines ---------------------------------

#[test]
fn a_malformed_line_is_skipped_not_fatal() {
    let events = parse_claude_line("not json at all {{{");

    assert_eq!(events, Vec::new());
}

#[test]
fn a_blank_line_is_skipped() {
    assert_eq!(parse_claude_line(""), Vec::new());
    assert_eq!(parse_claude_line("   "), Vec::new());
}

#[test]
fn a_user_envelope_produces_no_events() {
    let line = r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#;

    assert_eq!(parse_claude_line(line), Vec::new());
}

// -- opencode ---------------------------------------------------------------

#[test]
fn opencode_edit_tool_use_becomes_tool_used() {
    let line = r#"{"type":"tool_use","name":"edit","file_path":"src/main.rs"}"#;

    assert_eq!(
        parse_opencode_line(line),
        vec![ExecutorEvent::ToolUsed {
            tool: "edit".to_owned(),
            path: Some("src/main.rs".to_owned()),
        }]
    );
}

#[test]
fn opencode_ignores_tool_use_calls_that_are_not_edit() {
    let line = r#"{"type":"tool_use","name":"bash","command":"ls"}"#;

    assert_eq!(parse_opencode_line(line), Vec::new());
}

#[test]
fn opencode_question_becomes_question_asked() {
    let line = r#"{"type":"question","text":"Which port?"}"#;

    assert_eq!(
        parse_opencode_line(line),
        vec![ExecutorEvent::QuestionAsked {
            prompt: "Which port?".to_owned(),
        }]
    );
}

#[test]
fn an_opencode_malformed_line_is_skipped() {
    assert_eq!(parse_opencode_line("{not json"), Vec::new());
}
