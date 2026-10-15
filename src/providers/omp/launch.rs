use crate::infra::proc::Command;
use crate::providers::{Capabilities, LaunchContract};

const CAPABILITIES: Capabilities = Capabilities {
    supports_resume: true,
    supports_review: false,
    interactive: true,
    initial_prompt: true,
    resume_prompt: true,
};

const CONTRACT: LaunchContract = LaunchContract {
    binary: "omp",
    capabilities: CAPABILITIES,
};

#[must_use]
pub const fn contract() -> LaunchContract {
    CONTRACT
}

pub fn start_command(resume: bool) -> Command {
    super::super::resumable_start_command(CONTRACT.binary, resume)
}

/// OMP's positional initial message, after `--` so leading dashes stay text.
#[must_use]
pub fn prompt_args(prompt: &str) -> Vec<String> {
    vec!["--".to_owned(), prompt.to_owned()]
}
