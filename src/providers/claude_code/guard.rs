use camino::Utf8PathBuf;
use serde::Deserialize;

use crate::domain::guard::{GuardDecision, GuardOutcome, ToolRequest};
use crate::error::Failure;

/// Claude Code hook input: `tool_name`, `tool_input`, `cwd`, and the ids that
/// key instruction delivery (`agent_id` is present only inside subagents).
#[derive(Debug, Deserialize)]
struct ClaudeHookInput {
    tool_name: String,
    tool_input: serde_json::Value,
    cwd: Option<Utf8PathBuf>,
    session_id: Option<String>,
    agent_id: Option<String>,
    tool_use_id: Option<String>,
}

pub(crate) fn parse_tool_request(
    stdin_json: &str,
) -> Result<(ToolRequest, Option<Utf8PathBuf>), Failure> {
    let input: ClaudeHookInput = serde_json::from_str(stdin_json)
        .map_err(|e| Failure::blocked("guard.parse", format!("invalid Claude hook JSON: {e}")))?;
    let mut targets = Vec::new();
    if let Some(p) = input.tool_input.get("file_path").and_then(|v| v.as_str()) {
        targets.push(Utf8PathBuf::from(p));
    }
    let writes = crate::domain::guard::is_structured_write(&input.tool_name);
    let req = ToolRequest {
        search_pattern: crate::providers::extract_search_pattern(
            &input.tool_name,
            &input.tool_input,
        ),
        tool: input.tool_name,
        targets,
        writes,
        input: input.tool_input,
        agent: input.agent_id.or(input.session_id),
        call_id: input.tool_use_id,
    };
    Ok((req, input.cwd))
}

/// Claude reads the decision from JSON and always exits 0. Repository
/// instructions ride on an allow as `additionalContext`; a deny never
/// carries them.
pub(crate) fn render_decision(decision: &GuardDecision, context: Option<&str>) -> GuardOutcome {
    let (perm, reason, context): (&str, &str, Option<&str>) = match decision {
        GuardDecision::Allow => ("allow", "", context),
        GuardDecision::Deny { reason } => ("deny", reason.as_str(), None),
    };
    let mut output = serde_json::json!({
        "hookEventName": "PreToolUse",
        "permissionDecision": perm,
        "permissionDecisionReason": reason,
    });
    if let Some(context) = context
        && let Some(fields) = output.as_object_mut()
    {
        fields.insert("additionalContext".to_owned(), context.into());
    }
    GuardOutcome {
        body: serde_json::json!({ "hookSpecificOutput": output }).to_string(),
        exit_zero: true,
    }
}

/// The extra `--slice` hook entries: context only, never a
/// `permissionDecision`, so the guard entry's deny still blocks the call.
pub(crate) fn render_context(context: Option<&str>) -> GuardOutcome {
    let body = match context {
        Some(context) => serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "additionalContext": context,
            }
        }),
        None => serde_json::json!({}),
    };
    GuardOutcome {
        body: body.to_string(),
        exit_zero: true,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/providers/claude_code/guard.rs"]
mod tests;
