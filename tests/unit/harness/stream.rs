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
//
// Every line below is a verbatim capture from `opencode run --format json` on
// opencode 1.18.16, trimmed only of payload fields this module never reads.

/// The shape that matters: the tool's name is `part.tool` and its arguments
/// are `part.state.input`, with the path spelled `filePath`. Nothing lives at
/// the top level but the envelope.
#[test]
fn an_opencode_tool_use_is_read_out_of_its_part() {
    let line = r#"{"type":"tool_use","timestamp":1786406192677,"sessionID":"ses_011e4dff",
        "part":{"type":"tool","tool":"read","callID":"call_t45nbY","id":"prt_fee1b4464",
        "sessionID":"ses_011e4dff","state":{"status":"completed",
        "input":{"filePath":"/tmp/ocprobe/a.txt","limit":1},"output":"1: hello ivar",
        "title":"a.txt","time":{"start":1786406192663,"end":1786406192673}}}}"#;

    assert_eq!(
        parse_opencode_line(line),
        vec![
            ExecutorEvent::NativeSession {
                id: "ses_011e4dff".to_owned(),
            },
            ExecutorEvent::ToolUsed {
                tool: "read".to_owned(),
                path: Some("/tmp/ocprobe/a.txt".to_owned()),
            },
        ]
    );
}

/// The bug this rewrite undoes: the old parser matched `name` and `file_path`
/// at the top level, and only for `edit`. OpenCode emits neither key, so that
/// shape must now match nothing.
#[test]
fn the_old_top_level_opencode_tool_shape_matches_nothing() {
    let line = r#"{"type":"tool_use","name":"edit","file_path":"src/main.rs"}"#;

    assert_eq!(parse_opencode_line(line), Vec::new());
}

/// Every tool is journalled, not just `edit` — and a tool that names no file
/// still counts as activity.
#[test]
fn an_opencode_tool_without_a_path_is_still_a_tool_use() {
    let line = r#"{"type":"tool_use","timestamp":1,"sessionID":"ses_a",
        "part":{"type":"tool","tool":"bash","state":{"status":"completed",
        "input":{"command":"ls"}}}}"#;

    assert_eq!(
        parse_opencode_line(line),
        vec![
            ExecutorEvent::NativeSession {
                id: "ses_a".to_owned(),
            },
            ExecutorEvent::ToolUsed {
                tool: "bash".to_owned(),
                path: None,
            },
        ]
    );
}

/// A failed call — this hall's own execution guard refusing a write, say — is
/// something that happened, so it is journalled rather than filtered. The
/// error branch replaces `state` wholesale, so there is no `input` to read a
/// path out of.
#[test]
fn an_errored_opencode_tool_call_is_still_reported() {
    let line = r#"{"type":"tool_use","timestamp":1,"sessionID":"ses_a",
        "part":{"type":"tool","tool":"edit","state":{"status":"error",
        "error":"ivar denied write to /etc/passwd"}}}"#;

    assert_eq!(
        parse_opencode_line(line),
        vec![
            ExecutorEvent::NativeSession {
                id: "ses_a".to_owned(),
            },
            ExecutorEvent::ToolUsed {
                tool: "edit".to_owned(),
                path: None,
            },
        ]
    );
}

/// `opencode run` denies the `question` permission at session creation and its
/// JSON writer has no question envelope, so a question the model wants to ask
/// arrives as ordinary prose in a `text` part. Blocking on that would stall
/// every run on its first sentence — see "Assistant prose is not a question".
#[test]
fn opencode_prose_is_never_a_question() {
    let line = r#"{"type":"text","timestamp":1786406195392,"sessionID":"ses_011e3481",
        "part":{"id":"prt_fee1b4faf","messageID":"msg_fee1b4683","sessionID":"ses_011e3481",
        "type":"text","text":"Which colour do you prefer, red or blue?",
        "time":{"start":1786406195119,"end":1786406195389}}}"#;

    assert_eq!(
        parse_opencode_line(line),
        vec![ExecutorEvent::NativeSession {
            id: "ses_011e3481".to_owned(),
        }]
    );
}

/// The shape the old parser invented. OpenCode 1.18.16 emits no such
/// envelope, and no line of any shape may produce a `QuestionAsked`.
#[test]
fn no_opencode_line_produces_a_question_asked() {
    let lines = [
        r#"{"type":"question","text":"Which port?"}"#,
        r#"{"type":"permission.asked","sessionID":"ses_a","permission":"question"}"#,
        r#"{"type":"tool_use","sessionID":"ses_a","part":{"type":"tool","tool":"question",
            "state":{"status":"error","error":"Permission denied: question"}}}"#,
    ];

    for line in lines {
        assert!(
            !parse_opencode_line(line)
                .iter()
                .any(|event| matches!(event, ExecutorEvent::QuestionAsked { .. })),
            "was: {line}"
        );
    }
}

// -- opencode: the native session id ----------------------------------------

/// OpenCode stamps `sessionID` on every line rather than announcing it once,
/// so every parsed line carries it and the drain loop keeps the first.
#[test]
fn every_opencode_line_carries_the_native_session_id() {
    let line = r#"{"type":"step_start","timestamp":1786406190723,"sessionID":"ses_011e4dff",
        "part":{"id":"prt_fee1b3e7e","messageID":"msg_fee1b24ed","sessionID":"ses_011e4dff",
        "type":"step-start"}}"#;

    assert_eq!(
        parse_opencode_line(line),
        vec![ExecutorEvent::NativeSession {
            id: "ses_011e4dff".to_owned(),
        }]
    );
}

// -- opencode: errors and malformed lines -----------------------------------

/// In `--format json` nothing is written to stderr, so `exited 1` on its own
/// would be the whole story. The `error` envelope carries the reason.
#[test]
fn an_opencode_error_envelope_becomes_a_failure() {
    let line = r#"{"type":"error","timestamp":1,"sessionID":"ses_a",
        "error":{"name":"ProviderAuthError","data":{"message":"missing credentials"}}}"#;

    assert_eq!(
        parse_opencode_line(line),
        vec![
            ExecutorEvent::NativeSession {
                id: "ses_a".to_owned(),
            },
            ExecutorEvent::Failed {
                error: "missing credentials".to_owned(),
            },
        ]
    );
}

#[test]
fn an_opencode_error_falls_back_to_its_name() {
    let line = r#"{"type":"error","sessionID":"ses_a","error":{"name":"UnknownError"}}"#;

    assert_eq!(
        parse_opencode_line(line),
        vec![
            ExecutorEvent::NativeSession {
                id: "ses_a".to_owned(),
            },
            ExecutorEvent::Failed {
                error: "UnknownError".to_owned(),
            },
        ]
    );
}

#[test]
fn an_opencode_malformed_line_is_skipped() {
    assert_eq!(parse_opencode_line("{not json"), Vec::new());
    assert_eq!(parse_opencode_line(""), Vec::new());
    assert_eq!(parse_opencode_line("   "), Vec::new());
}
