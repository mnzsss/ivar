#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::provider::Provider;

#[test]
fn claude_session_start_prompt_tool_failure_and_end_are_parsed() {
    let started = parse_capture(
        Provider::ClaudeCode,
        r#"{"hook_event_name":"SessionStart","session_id":"n1","cwd":"/hall","source":"startup"}"#,
    )
    .unwrap();
    assert_eq!(started.kind, CaptureKind::Started);
    assert_eq!(started.native_session_id, "n1");
    assert_eq!(started.cwd.as_deref().map(|p| p.as_str()), Some("/hall"));

    let prompted = parse_capture(
        Provider::ClaudeCode,
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"n1","prompt":"fix the bug"}"#,
    )
    .unwrap();
    assert_eq!(
        prompted.kind,
        CaptureKind::Prompted {
            text: "fix the bug".to_owned()
        }
    );

    let tool = parse_capture(
        Provider::ClaudeCode,
        r#"{"hook_event_name":"PostToolUse","session_id":"n1","tool_name":"Bash","tool_input":{"command":"ls"},"tool_response":{"stdout":"a"}}"#,
    )
    .unwrap();
    assert!(
        matches!(tool.kind, CaptureKind::Tool { ref name, failed: false, .. } if name == "Bash")
    );

    let failed = parse_capture(
        Provider::ClaudeCode,
        r#"{"hook_event_name":"PostToolUseFailure","session_id":"n1","tool_name":"Bash","tool_input":{},"error":"exit 1"}"#,
    )
    .unwrap();
    assert!(matches!(
        failed.kind,
        CaptureKind::Tool { failed: true, .. }
    ));

    let ended = parse_capture(
        Provider::ClaudeCode,
        r#"{"hook_event_name":"SessionEnd","session_id":"n1","reason":"logout"}"#,
    )
    .unwrap();
    assert_eq!(
        ended.kind,
        CaptureKind::Ended {
            reason: Some("logout".to_owned())
        }
    );
}

#[test]
fn t3_parses_exactly_like_claude_code() {
    let payload = r#"{"hook_event_name":"UserPromptSubmit","session_id":"n1","prompt":"hi"}"#;
    assert_eq!(
        parse_capture(Provider::T3, payload).unwrap(),
        parse_capture(Provider::ClaudeCode, payload).unwrap()
    );
}

#[test]
fn the_in_process_providers_share_the_wire_shape() {
    let payload = r#"{"event":"tool","session":"s1","cwd":"/hall","tool":"edit","args":{"filePath":"a.rs"},"failed":true,"result":"boom"}"#;
    for provider in [Provider::OpenCode, Provider::Omp] {
        let event = parse_capture(provider, payload).unwrap();
        assert!(
            matches!(event.kind, CaptureKind::Tool { ref name, failed: true, .. } if name == "edit")
        );
        assert_eq!(event.native_session_id, "s1");
    }
}

#[test]
fn garbage_and_unknown_events_are_skipped_not_errors() {
    assert!(parse_capture(Provider::ClaudeCode, "not json").is_err());
    assert!(
        parse_capture(
            Provider::ClaudeCode,
            r#"{"hook_event_name":"Stop","session_id":"n1"}"#
        )
        .is_err()
    );
    assert!(parse_capture(Provider::Omp, r#"{"event":"idle","session":"s"}"#).is_err());
    assert!(
        parse_capture(
            Provider::ClaudeCode,
            r#"{"hook_event_name":"SessionStart"}"#
        )
        .is_err()
    );
}

#[test]
fn debug_of_capture_events_never_prints_prompts_inputs_or_results() {
    let prompted = CaptureEvent {
        kind: CaptureKind::Prompted {
            text: "secret-prompt-text".to_owned(),
        },
        native_session_id: "n1".to_owned(),
        cwd: None,
    };
    let tool = CaptureKind::Tool {
        name: "Write".to_owned(),
        input: serde_json::json!({ "content": "secret-input-text" }),
        failed: false,
        result: Some("secret-result-text".to_owned()),
    };

    let printed = format!("{prompted:?} {tool:?}");

    for secret in [
        "secret-prompt-text",
        "secret-input-text",
        "secret-result-text",
    ] {
        assert!(!printed.contains(secret), "{printed}");
    }
    assert!(printed.contains("Write"), "{printed}");
}
