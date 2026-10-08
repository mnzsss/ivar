//! Provider-neutral guard vocabulary: what a tool asked for, what the guard
//! decided, and how that decision leaves the process.
//!
//! These live in `domain` rather than beside the decision logic because the
//! per-provider adapters in `src/providers/` must name them, and `providers`
//! may not import `action` (`tests/static_checks/architecture.rs`).

use camino::Utf8PathBuf;

/// A tool invocation the guard is asked to evaluate.
#[derive(Debug, Clone)]
pub struct ToolRequest {
    pub tool: String,
    pub targets: Vec<Utf8PathBuf>,
    pub writes: bool,
    pub search_pattern: Option<String>,
    /// The provider's raw tool input (Claude `tool_input`, omp/opencode `args`).
    pub input: serde_json::Value,
    /// Whose delivery state this call belongs to: Claude `agent_id` else
    /// `session_id`; omp/opencode payload `agent`.
    pub agent: Option<String>,
    /// Claude `tool_use_id`; `None` for omp and opencode.
    pub call_id: Option<String>,
}

/// Normalise tool name and check if it is a standard structured write tool.
#[must_use]
pub fn is_structured_write(tool: &str) -> bool {
    let normalised: String = tool
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    matches!(
        normalised.as_str(),
        "write" | "edit" | "multiedit" | "notebookedit" | "applypatch" | "patch"
    )
}

/// The guard's decision for a tool request.
#[derive(Debug)]
pub enum GuardDecision {
    Allow,
    Deny { reason: String },
}

/// The outcome of a guard evaluation: stdout body and whether the process
/// exits 0.
#[derive(Debug)]
pub struct GuardOutcome {
    pub body: String,
    pub exit_zero: bool,
}
