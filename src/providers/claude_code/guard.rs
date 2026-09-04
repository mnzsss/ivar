use camino::Utf8PathBuf;
use serde::Deserialize;

use crate::domain::guard::{GuardDecision, GuardOutcome, ToolRequest};
use crate::error::Failure;

/// Claude Code hook input: `tool_name`, `tool_input.file_path`, `cwd`.
#[derive(Debug, Deserialize)]
struct ClaudeHookInput {
    tool_name: String,
    tool_input: serde_json::Value,
    cwd: Option<Utf8PathBuf>,
}

pub(crate) fn parse_tool_request(
    stdin_json: &str,
) -> Result<(ToolRequest, Option<Utf8PathBuf>), Failure> {
    let input: ClaudeHookInput = serde_json::from_str(stdin_json)
        .map_err(|e| Failure::blocked("guard.parse", format!("invalid Claude hook JSON: {e}")))?;
    let req = ToolRequest {
        tool: input.tool_name,
        file_path: input
            .tool_input
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(Utf8PathBuf::from),
    };
    Ok((req, input.cwd))
}

pub(crate) fn render_decision(decision: &GuardDecision) -> GuardOutcome {
    let (perm, reason): (String, String) = match decision {
        GuardDecision::Allow => ("allow".into(), String::new()),
        GuardDecision::Deny { reason } => ("deny".into(), reason.clone()),
    };
    let body = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": perm,
            "permissionDecisionReason": reason,
        }
    });
    GuardOutcome {
        body: body.to_string(),
        exit_zero: true,
    }
}
