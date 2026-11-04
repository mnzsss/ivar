use camino::Utf8PathBuf;
use serde::Deserialize;

use crate::providers::capture::{CaptureEvent, CaptureKind, ParseSkip};

#[derive(Deserialize, Default)]
#[serde(default)]
struct ClaudeCapture {
    hook_event_name: Option<String>,
    session_id: Option<String>,
    cwd: Option<Utf8PathBuf>,
    prompt: Option<String>,
    tool_name: Option<String>,
    tool_input: Option<serde_json::Value>,
    tool_response: Option<serde_json::Value>,
    error: Option<String>,
    reason: Option<String>,
}

pub(crate) fn parse(stdin: &str) -> Result<CaptureEvent, ParseSkip> {
    let payload: ClaudeCapture =
        serde_json::from_str(stdin).map_err(|_| ParseSkip("invalid payload"))?;
    let native_session_id = payload.session_id.ok_or(ParseSkip("no session id"))?;
    let failed = payload.hook_event_name.as_deref() == Some("PostToolUseFailure");
    let kind = match payload.hook_event_name.as_deref() {
        Some("SessionStart") => CaptureKind::Started,
        Some("UserPromptSubmit") => CaptureKind::Prompted {
            text: payload.prompt.ok_or(ParseSkip("no prompt"))?,
        },
        Some("PostToolUse" | "PostToolUseFailure") => CaptureKind::Tool {
            name: payload.tool_name.ok_or(ParseSkip("no tool name"))?,
            input: payload.tool_input.unwrap_or_default(),
            failed,
            result: if failed {
                payload.error
            } else {
                payload.tool_response.map(|response| response.to_string())
            },
        },
        Some("SessionEnd") => CaptureKind::Ended {
            reason: payload.reason,
        },
        _ => return Err(ParseSkip("unsupported event")),
    };
    Ok(CaptureEvent {
        kind,
        native_session_id,
        cwd: payload.cwd,
    })
}
