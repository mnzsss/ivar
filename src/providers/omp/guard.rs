use camino::Utf8PathBuf;
use serde::Deserialize;

use crate::domain::guard::{GuardDecision, GuardOutcome, ToolRequest};
use crate::error::Failure;

/// OMP hook input, as the embedded hook actually sends it
/// (`src/providers/omp/hook.rs`): `{ tool, args, cwd }`. It is the OpenCode
/// wire shape, because the hook was written to it — the field names OMP uses
/// internally (`toolName`, `input`) are translated JS-side before the spawn.
#[derive(Debug, Deserialize)]
pub(crate) struct OmpHookInput {
    pub(crate) tool: String,
    #[serde(default)]
    pub(crate) args: serde_json::Value,
    pub(crate) cwd: Option<Utf8PathBuf>,
}

pub(crate) fn parse_tool_request(
    stdin_json: &str,
) -> Result<(ToolRequest, Option<Utf8PathBuf>), Failure> {
    let input: OmpHookInput = serde_json::from_str(stdin_json)
        .map_err(|e| Failure::blocked("guard.parse", format!("invalid OMP hook JSON: {e}")))?;
    let file_path = input
        .args
        .get("filePath")
        .or_else(|| input.args.get("file_path"))
        .or_else(|| input.args.get("path"))
        .and_then(|v| v.as_str())
        .map(Utf8PathBuf::from);
    Ok((
        ToolRequest {
            tool: input.tool,
            file_path,
        },
        input.cwd,
    ))
}

/// The hook detects a block by `execFileSync` throwing, which happens only on
/// a non-zero exit, and reads the reason from stdout. So deny must exit
/// non-zero with the bare reason on stdout — the same shape OpenCode uses.
/// A JSON `{"block":true}` body with exit 0 would be silently ignored: the
/// hook would never enter its `catch`, and every denied write would proceed.
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
