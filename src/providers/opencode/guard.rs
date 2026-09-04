use camino::Utf8PathBuf;
use serde::Deserialize;

use crate::domain::guard::{GuardDecision, GuardOutcome, ToolRequest};
use crate::error::Failure;

/// OpenCode hook input: `tool`, `args.filePath`, `cwd`.
#[derive(Debug, Deserialize)]
struct OpenCodeHookInput {
    tool: String,
    args: serde_json::Value,
    cwd: Option<Utf8PathBuf>,
}

pub(crate) fn parse_tool_request(
    stdin_json: &str,
) -> Result<(ToolRequest, Option<Utf8PathBuf>), Failure> {
    let input: OpenCodeHookInput = serde_json::from_str(stdin_json)
        .map_err(|e| Failure::blocked("guard.parse", format!("invalid OpenCode hook JSON: {e}")))?;
    let req = ToolRequest {
        tool: input.tool,
        file_path: input
            .args
            .get("filePath")
            .and_then(|v| v.as_str())
            .map(Utf8PathBuf::from),
    };
    Ok((req, input.cwd))
}

pub(crate) fn render_decision(decision: &GuardDecision) -> GuardOutcome {
    match decision {
        GuardDecision::Allow => GuardOutcome {
            body: String::new(),
            exit_zero: true,
        },
        GuardDecision::Deny { reason } => GuardOutcome {
            body: reason.clone(),
            exit_zero: false,
        },
    }
}
