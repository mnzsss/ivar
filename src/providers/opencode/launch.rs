use crate::infra::proc::Command;
use crate::providers::{Capabilities, LaunchContract};

const CAPABILITIES: Capabilities = Capabilities {
    supports_resume: true,
    supports_review: false,
    interactive: true,
    initial_prompt: true,
};

const CONTRACT: LaunchContract = LaunchContract {
    binary: "opencode",
    capabilities: CAPABILITIES,
};

#[must_use]
pub const fn contract() -> LaunchContract {
    CONTRACT
}

pub fn start_command(resume: bool) -> Command {
    super::super::resumable_start_command(CONTRACT.binary, resume)
}

/// OpenCode's `--prompt`, as one argv so yargs never reads the text as a flag.
#[must_use]
pub fn prompt_args(prompt: &str) -> Vec<String> {
    vec![format!("--prompt={prompt}")]
}
