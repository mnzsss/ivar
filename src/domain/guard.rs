//! Provider-neutral guard vocabulary: what a tool asked for, what the guard
//! decided, and how that decision leaves the process.
//!
//! These live in `domain` rather than beside the decision logic because the
//! per-provider adapters in `src/providers/` must name them, and `providers`
//! may not import `action` (`tests/architecture.rs`).

use camino::Utf8PathBuf;

/// A tool invocation the guard is asked to evaluate.
#[derive(Debug)]
pub struct ToolRequest {
    pub tool: String,
    pub file_path: Option<Utf8PathBuf>,
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
